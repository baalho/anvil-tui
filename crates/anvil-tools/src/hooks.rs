//! Hook system for running scripts before/after tool execution.
//!
//! # How it works
//! Hooks are scripts in `.anvil/hooks/` named by convention:
//! - `pre-shell.{sh,ps1,cmd}` — runs before every shell command
//! - `post-edit.{sh,ps1,cmd}` — runs after every file edit (file_write, file_edit)
//! - `pre-{tool}.{ext}` / `post-{tool}.{ext}` — per-tool hooks
//!
//! # Platform support
//! Scripts are discovered by extension in platform-preferred order:
//! - Unix: `.sh` → `.ps1` → `.cmd` → `.bat`
//! - Windows: `.ps1` → `.cmd` → `.bat` → `.sh`
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
        let script = match find_hook_script(&self.hooks_dir, hook_name) {
            Some(path) => path,
            None => {
                return HookResult {
                    ran: false,
                    success: true,
                    output: String::new(),
                }
            }
        };

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

/// Search for a hook script with platform-appropriate extensions.
/// Priority: .sh on unix, .ps1/.cmd/.bat on Windows, then cross-platform fallbacks.
fn find_hook_script(hooks_dir: &Path, hook_name: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    const EXTENSIONS: &[&str] = &["sh", "ps1", "cmd", "bat"];
    #[cfg(windows)]
    const EXTENSIONS: &[&str] = &["ps1", "cmd", "bat", "sh"];

    for ext in EXTENSIONS {
        let path = hooks_dir.join(format!("{hook_name}.{ext}"));
        if path.exists() {
            return Some(path);
        }
    }
    None
}

async fn run_script(script: &Path) -> Result<(bool, String)> {
    let ext = script.extension().and_then(|e| e.to_str()).unwrap_or("sh");

    let mut cmd = match ext {
        "ps1" => {
            // Use pwsh (cross-platform PowerShell) if available, fall back to powershell
            let shell = if which_exists("pwsh") {
                "pwsh"
            } else {
                "powershell"
            };
            let mut c = tokio::process::Command::new(shell);
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
            c.arg(script);
            c
        }
        "cmd" | "bat" => {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/C"]);
            c.arg(script);
            c
        }
        _ => {
            // .sh or unknown — use sh
            let mut c = tokio::process::Command::new("sh");
            c.arg(script);
            c
        }
    };

    let output = cmd.output().await?;

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

/// Check if a command exists on PATH.
fn which_exists(cmd: &str) -> bool {
    // `which` on unix, `where` on Windows — but both just need to find the binary.
    // Trying to run the command with --version is fragile, so we check PATH directly.
    if let Ok(path) = std::env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        let exts: &[&str] = if cfg!(windows) {
            &["", ".exe", ".cmd", ".bat"]
        } else {
            &[""]
        };
        for dir in path.split(sep) {
            for ext in exts {
                let candidate = PathBuf::from(dir).join(format!("{cmd}{ext}"));
                if candidate.is_file() {
                    return true;
                }
            }
        }
    }
    false
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

    #[test]
    fn find_hook_prefers_platform_extension() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        std::fs::write(hooks_dir.join("pre-shell.sh"), "").unwrap();
        std::fs::write(hooks_dir.join("pre-shell.ps1"), "").unwrap();

        let found = find_hook_script(&hooks_dir, "pre-shell").unwrap();
        #[cfg(unix)]
        assert!(found.extension().unwrap() == "sh");
        #[cfg(windows)]
        assert!(found.extension().unwrap() == "ps1");
    }

    #[test]
    fn find_hook_falls_back_to_other_extension() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        // Only a .ps1 file on unix — should still be found as fallback
        std::fs::write(hooks_dir.join("pre-shell.ps1"), "").unwrap();

        let found = find_hook_script(&hooks_dir, "pre-shell");
        assert!(found.is_some());
        assert!(found.unwrap().extension().unwrap() == "ps1");
    }

    #[test]
    fn find_hook_returns_none_when_missing() {
        let dir = TempDir::new().unwrap();
        let hooks_dir = dir.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();

        assert!(find_hook_script(&hooks_dir, "pre-shell").is_none());
    }
}
