//! Tool executor — dispatches tool calls to implementations and manages env passthrough.
//!
//! # How env passthrough works
//! The shell tool uses `env_clear()` for security, only passing safe vars (PATH, HOME, etc.).
//! When a skill declares `env: [DOCKER_HOST]` in its frontmatter, the agent calls
//! `set_extra_env()` to add those vars to the passthrough list. The shell tool then
//! includes them alongside the base safe vars.
//!
//! This is scoped to the session — deactivating a skill removes its env vars.

use crate::permission::PermissionHandler;
use crate::tools;
use crate::truncation::{self, TruncationConfig};
use anyhow::{bail, Result};
use serde_json::Value;
use std::path::Path;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Kids sandbox configuration — restricts workspace and shell commands.
#[derive(Debug, Clone)]
pub struct KidsSandbox {
    /// Restricted workspace path (overrides general workspace for file tools).
    pub workspace: PathBuf,
    /// Allowed shell commands (first word of command string).
    pub allowed_commands: Vec<String>,
}

/// Default safe commands for kids mode.
pub const DEFAULT_KIDS_COMMANDS: &[&str] = &[
    "echo", "cat", "ls", "python3", "python", "cargo", "rustc", "node", "npm", "git",
];

/// Executes tool calls from the LLM, enforcing workspace boundaries and output limits.
///
/// Uses `Arc` for shared state so the executor can be cloned cheaply for
/// parallel read-only tool execution (each spawn gets its own handle to
/// the same permission state and config).
#[derive(Clone)]
pub struct ToolExecutor {
    workspace: PathBuf,
    permissions: Arc<PermissionHandler>,
    shell_timeout: u64,
    truncation_config: TruncationConfig,
    /// Extra environment variables to pass through to shell commands.
    /// Populated from active skills' `env` frontmatter declarations.
    extra_env: Vec<String>,
    /// Cache of file contents keyed by canonical path.
    /// Invalidated when file_write or file_edit modifies a file.
    file_cache: Arc<Mutex<HashMap<PathBuf, String>>>,
    /// Kids sandbox — when active, restricts workspace and shell commands.
    kids_sandbox: Arc<Mutex<Option<KidsSandbox>>>,
    /// Write ledger — tracks files modified by the agent to prevent
    /// watcher feedback loops. Optional: only set in daemon/watch mode.
    write_ledger: Option<crate::ledger::WriteLedger>,
}

impl ToolExecutor {
    /// Create a new executor for the given workspace.
    pub fn new(workspace: PathBuf, shell_timeout: u64, output_limit: usize) -> Self {
        Self {
            workspace,
            permissions: Arc::new(PermissionHandler::new()),
            shell_timeout,
            truncation_config: TruncationConfig {
                max_lines: 200,
                max_bytes: output_limit.max(30_000),
            },
            extra_env: Vec::new(),
            file_cache: Arc::new(Mutex::new(HashMap::new())),
            kids_sandbox: Arc::new(Mutex::new(None)),
            write_ledger: None,
        }
    }

    /// Set the write ledger for tracking agent file modifications.
    /// Called by the binary crate when running in daemon or watch mode.
    pub fn set_write_ledger(&mut self, ledger: crate::ledger::WriteLedger) {
        self.write_ledger = Some(ledger);
    }

    /// Get the permission handler for checking/granting tool permissions.
    pub fn permissions(&self) -> &PermissionHandler {
        &self.permissions
    }

    /// Set extra environment variables to pass through to shell commands.
    ///
    /// # Why this exists
    /// Skills like Docker need `DOCKER_HOST`, server admin needs `SSH_AUTH_SOCK`.
    /// These are declared in skill frontmatter and activated when the skill loads.
    /// The shell tool merges these with the base safe vars (PATH, HOME, etc.).
    pub fn set_extra_env(&mut self, vars: Vec<String>) {
        self.extra_env = vars;
    }

    /// Get the currently configured extra env vars (for `/stats` display).
    pub fn extra_env(&self) -> &[String] {
        &self.extra_env
    }

    /// Activate kids sandbox mode with a restricted workspace and command allowlist.
    pub fn set_kids_sandbox(&self, sandbox: KidsSandbox) {
        *self.kids_sandbox.lock().unwrap() = Some(sandbox);
    }

    /// Deactivate kids sandbox mode.
    pub fn clear_kids_sandbox(&self) {
        *self.kids_sandbox.lock().unwrap() = None;
    }

    /// Get the effective workspace (kids sandbox workspace if active, else general).
    fn effective_workspace(&self) -> PathBuf {
        if let Some(sandbox) = self.kids_sandbox.lock().unwrap().as_ref() {
            sandbox.workspace.clone()
        } else {
            self.workspace.clone()
        }
    }

    /// Check if a shell command is allowed under the current sandbox.
    /// Returns Ok(()) if allowed, Err with friendly message if blocked.
    fn check_shell_allowlist(&self, command: &str) -> Result<()> {
        let sandbox = self.kids_sandbox.lock().unwrap();
        if let Some(sandbox) = sandbox.as_ref() {
            let first_word = command.split_whitespace().next().unwrap_or("");
            // Strip path prefix (e.g., /usr/bin/python3 -> python3)
            let binary = first_word.rsplit('/').next().unwrap_or(first_word);
            if !sandbox.allowed_commands.iter().any(|c| c == binary) {
                bail!(
                    "✨ That command isn't available in kids mode! Try one of: {}",
                    sandbox.allowed_commands.join(", ")
                );
            }
        }
        Ok(())
    }

    /// Execute a tool call and return the (possibly truncated) output.
    ///
    /// # Tool dispatch
    /// - `file_read`, `grep`, `ls`, `find` — read-only, no permission needed
    /// - `file_write`, `file_edit` — mutating, needs permission
    /// - `shell` — mutating, needs permission, uses env passthrough
    ///
    /// # Validation
    /// Required arguments are checked before dispatch. Missing or empty
    /// required args produce clear error messages that help the LLM
    /// self-correct on the next attempt.
    pub async fn execute(&self, tool_name: &str, args: &Value) -> Result<crate::ToolOutput> {
        // Validate required arguments before dispatch
        self.validate_args(tool_name, args)?;

        let workspace = self.effective_workspace();

        let result = match tool_name {
            "file_read" => tools::file_read(&workspace, args).await?,
            "file_write" => {
                let result = tools::file_write(&workspace, args).await?;
                self.invalidate_cache(args);
                self.record_write(&workspace, args);
                result
            }
            "file_edit" => {
                let result = tools::file_edit(&workspace, args).await?;
                self.invalidate_cache(args);
                self.record_write(&workspace, args);
                result
            }
            "shell" => {
                // Check allowlist before executing shell commands
                if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
                    self.check_shell_allowlist(cmd)?;
                }
                tools::shell(&workspace, args, self.shell_timeout, &self.extra_env).await?
            }
            "grep" => tools::grep(&workspace, args).await?,
            "ls" => tools::ls(&workspace, args).await?,
            "find" => tools::find(&workspace, args).await?,
            "git_status" => tools::git_status(&workspace, args).await?,
            "git_diff" => tools::git_diff(&workspace, args).await?,
            "git_log" => tools::git_log(&workspace, args).await?,
            "git_commit" => tools::git_commit(&workspace, args).await?,
            _ => bail!("unknown tool: {tool_name}"),
        };

        let truncated = truncation::truncate_output(&result, &self.truncation_config);
        Ok(crate::ToolOutput::Text(truncated.content))
    }

    /// Invalidate cached file content when a file is modified.
    fn invalidate_cache(&self, args: &Value) {
        if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
            let resolved = self.workspace.join(path_str);
            if let Ok(canonical) = resolved.canonicalize() {
                self.file_cache.lock().unwrap().remove(&canonical);
            }
        }
    }

    /// Record a file write in the ledger so the watcher can suppress
    /// the resulting filesystem event. No-op if no ledger is set.
    fn record_write(&self, workspace: &Path, args: &Value) {
        if let Some(ref ledger) = self.write_ledger {
            if let Some(path_str) = args.get("path").and_then(|v| v.as_str()) {
                let full_path = workspace.join(path_str);
                if let Ok(meta) = std::fs::metadata(&full_path) {
                    if let Ok(mtime) = meta.modified() {
                        ledger.record(full_path, mtime);
                    }
                }
            }
        }
    }

    /// Get the number of cached file entries (for `/stats` display).
    pub fn cache_size(&self) -> usize {
        self.file_cache.lock().unwrap().len()
    }

    /// Check that required arguments are present and non-empty.
    /// Returns actionable error messages so the LLM can self-correct.
    fn validate_args(&self, tool_name: &str, args: &Value) -> Result<()> {
        let required: &[&str] = match tool_name {
            "file_read" => &["path"],
            "file_write" => &["path", "content"],
            "file_edit" => &["path", "old_str"],
            "shell" => &["command"],
            "grep" => &["pattern", "path"],
            "ls" => &["path"],
            "find" => &[], // path and pattern have defaults
            "git_status" => &[],
            "git_diff" => &[],
            "git_log" => &[],
            "git_commit" => &["message"],
            _ => return Ok(()),
        };

        let mut missing = Vec::new();
        for &field in required {
            match args.get(field) {
                None => missing.push(field),
                Some(Value::Null) => missing.push(field),
                Some(Value::String(s)) if s.is_empty() && field != "old_str" => missing.push(field),
                _ => {}
            }
        }

        if !missing.is_empty() {
            bail!(
                "{tool_name}: missing required argument(s): {}. \
                 Please provide all required arguments.",
                missing.join(", ")
            );
        }

        Ok(())
    }
}
