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
use std::path::PathBuf;

/// Executes tool calls from the LLM, enforcing workspace boundaries and output limits.
///
/// Each tool call is dispatched to the appropriate implementation in `tools.rs`,
/// then the output is truncated if it exceeds configured limits.
pub struct ToolExecutor {
    workspace: PathBuf,
    permissions: PermissionHandler,
    shell_timeout: u64,
    truncation_config: TruncationConfig,
    /// Extra environment variables to pass through to shell commands.
    /// Populated from active skills' `env` frontmatter declarations.
    extra_env: Vec<String>,
}

impl ToolExecutor {
    /// Create a new executor for the given workspace.
    pub fn new(workspace: PathBuf, shell_timeout: u64, output_limit: usize) -> Self {
        Self {
            workspace,
            permissions: PermissionHandler::new(),
            shell_timeout,
            truncation_config: TruncationConfig {
                max_lines: 200,
                max_bytes: output_limit.max(30_000),
            },
            extra_env: Vec::new(),
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
    pub async fn execute(&self, tool_name: &str, args: &Value) -> Result<String> {
        let result = match tool_name {
            "file_read" => tools::file_read(&self.workspace, args).await?,
            "file_write" => tools::file_write(&self.workspace, args).await?,
            "file_edit" => tools::file_edit(&self.workspace, args).await?,
            "shell" => {
                tools::shell(&self.workspace, args, self.shell_timeout, &self.extra_env).await?
            }
            "grep" => tools::grep(&self.workspace, args).await?,
            "ls" => tools::ls(&self.workspace, args).await?,
            "find" => tools::find(&self.workspace, args).await?,
            _ => bail!("unknown tool: {tool_name}"),
        };

        let truncated = truncation::truncate_output(&result, &self.truncation_config);
        Ok(truncated.content)
    }
}
