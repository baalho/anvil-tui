//! Hook system for running scripts before/after tool execution.
//!
//! # How it works
//! Hooks are shell scripts in `.anvil/hooks/` named by convention:
//! - `pre-shell.sh` — runs before every shell command
//! - `post-edit.sh` — runs after every file edit (file_write, file_edit)
//! - `pre-{tool}.sh` / `post-{tool}.sh` — per-tool hooks
//!
//! # Failure behavior
//! By default, a failing pre-hook blocks the tool execution. This is
//! configurable via `block_on_failure` in the hook runner.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Runs pre/post hooks for tool execution.
#[derive(Debug, Clone)]
pub struct HookRunner {
    hooks_dir: PathBuf,
    /// If true, a failing pre-hook prevents the tool from executing.
    pub block_on_failure: bool,
}

/// Result of running a hook script.
#[derive(Debug)]
pub struct HookResult {
    pub ran: bool,
    pub success: bool,
    pub output: String,
}

impl HookRunner {
    pub fn new(hooks_dir: PathBuf) -> Self {
        Self {
            hooks_dir,
            block_on_failure: true,
        }
    }

    /// Run a pre-hook for the given tool. Returns the hook result.
    /// If no hook exists, returns `HookResult { ran: false, success: true, .. }`.
    pub async fn run_pre_hook(&self, tool_name: &str) -> HookResult {
        self.run_hook(&format!("pre-{tool_name}")).await
    }

    /// Run a post-hook for the given tool.
    pub async fn run_post_hook(&self, tool_name: &str) -> HookResult {
        self.run_hook(&format!("post-{tool_name}")).await
    }

    async fn run_hook(&self, hook_name: &str) -> HookResult {
        let script = self.hooks_dir.join(format!("{hook_name}.sh"));
        if !script.exists() {
            return HookResult {
                ran: false,
                success: true,
                output: String::new(),
            };
        }

        match run_script(&script).await {
            Ok((success, output)) => HookResult {
                ran: true,
                success,
                output,
            },
            Err(e) => HookResult {
                ran: true,
                success: false,
                output: format!("hook error: {e}"),
            },
        }
    }

    /// Check if a hooks directory exists and has any scripts.
    pub fn has_hooks(&self) -> bool {
        self.hooks_dir.is_dir()
    }
}

async fn run_script(script: &Path) -> Result<(bool, String)> {
    let output = tokio::process::Command::new("sh")
        .arg(script)
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    Ok((output.status.success(), combined))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn no_hook_returns_not_ran() {
        let dir = TempDir::new().unwrap();
        let runner = HookRunner::new(dir.path().join("hooks"));
        let result = runner.run_pre_hook("shell").await;
        assert!(!result.ran);
        assert!(result.success);
    }

    #[tokio::test]
    async fn pre_hook_runs_script() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        std::fs::write(hooks_dir.join("pre-shell.sh"), "#!/bin/sh\necho hook-ran").unwrap();

        let runner = HookRunner::new(hooks_dir);
        let result = runner.run_pre_hook("shell").await;
        assert!(result.ran);
        assert!(result.success);
        assert!(result.output.contains("hook-ran"));
    }

    #[tokio::test]
    async fn failing_hook_reports_failure() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        std::fs::write(hooks_dir.join("pre-shell.sh"), "#!/bin/sh\nexit 1").unwrap();

        let runner = HookRunner::new(hooks_dir);
        let result = runner.run_pre_hook("shell").await;
        assert!(result.ran);
        assert!(!result.success);
    }
}
