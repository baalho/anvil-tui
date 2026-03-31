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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
        }
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
    pub async fn execute(&self, tool_name: &str, args: &Value) -> Result<String> {
        // Validate required arguments before dispatch
        self.validate_args(tool_name, args)?;

        let result = match tool_name {
            "file_read" => tools::file_read(&self.workspace, args).await?,
            "file_write" => {
                let result = tools::file_write(&self.workspace, args).await?;
                self.invalidate_cache(args);
                result
            }
            "file_edit" => {
                let result = tools::file_edit(&self.workspace, args).await?;
                self.invalidate_cache(args);
                result
            }
            "shell" => {
                tools::shell(&self.workspace, args, self.shell_timeout, &self.extra_env).await?
            }
            "grep" => tools::grep(&self.workspace, args).await?,
            "ls" => tools::ls(&self.workspace, args).await?,
            "find" => tools::find(&self.workspace, args).await?,
            "git_status" => tools::git_status(&self.workspace, args).await?,
            "git_diff" => tools::git_diff(&self.workspace, args).await?,
            "git_log" => tools::git_log(&self.workspace, args).await?,
            "git_commit" => tools::git_commit(&self.workspace, args).await?,
            _ => bail!("unknown tool: {tool_name}"),
        };

        let truncated = truncation::truncate_output(&result, &self.truncation_config);
        Ok(truncated.content)
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
