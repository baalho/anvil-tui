//! Zellij pane integration — send artifacts to floating panes.
//!
//! When running inside a Zellij session, long tool output, diffs, and
//! errors can be sent to floating panes instead of cluttering the chat.
//! All operations are best-effort — Zellij CLI failures fall back
//! silently to inline rendering.

use std::io::Write;
use std::process::{Command, Stdio};

/// Helper for programmatic Zellij pane control.
///
/// Uses `zellij action` CLI commands. All methods are best-effort —
/// failures are logged but never propagated to the caller.
pub struct ZellijPanes;

impl ZellijPanes {
    /// Whether we're running inside a Zellij session.
    pub fn is_available() -> bool {
        std::env::var("ZELLIJ").is_ok()
    }

    /// Open a floating pane with the given content and title.
    /// Returns true if the pane was opened successfully.
    pub fn open_floating_pane(title: &str, content: &str) -> bool {
        if !Self::is_available() {
            return false;
        }

        // Strategy: pipe content through `zellij action new-pane --floating`
        // with a command that displays it. We use `less` for scrollable output.
        // If that fails, fall back to `cat`.
        let script = format!("echo {} | less -R", shell_escape(content));

        let result = Command::new("zellij")
            .args([
                "action",
                "new-pane",
                "--floating",
                "--name",
                title,
                "--",
                "sh",
                "-c",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match result {
            Ok(status) => status.success(),
            Err(e) => {
                tracing::debug!("zellij pane failed: {e}");
                false
            }
        }
    }

    /// Write content to a temporary file and open it in a floating pane.
    /// More reliable than piping for large content.
    pub fn open_pane_with_file(title: &str, content: &str) -> bool {
        if !Self::is_available() {
            return false;
        }

        // Write to a temp file, then open it in a pane with less
        let tmp = match tempfile(content) {
            Some(p) => p,
            None => return false,
        };

        let result = Command::new("zellij")
            .args([
                "action",
                "new-pane",
                "--floating",
                "--name",
                title,
                "--",
                "less",
                "-R",
                &tmp,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match result {
            Ok(status) => status.success(),
            Err(e) => {
                tracing::debug!("zellij pane failed: {e}");
                false
            }
        }
    }
}

/// Write content to a temp file and return the path.
fn tempfile(content: &str) -> Option<String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("anvil-pane-{}.txt", std::process::id()));
    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(content.as_bytes()).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// Escape content for shell embedding in single quotes.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_available_returns_false_without_env() {
        // In test environment, ZELLIJ is not set
        std::env::remove_var("ZELLIJ");
        assert!(!ZellijPanes::is_available());
    }

    #[test]
    fn shell_escape_handles_quotes() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn open_pane_returns_false_outside_zellij() {
        std::env::remove_var("ZELLIJ");
        assert!(!ZellijPanes::open_floating_pane("test", "content"));
    }

    #[test]
    fn tempfile_creates_file() {
        let path = tempfile("test content").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "test content");
        std::fs::remove_file(&path).ok();
    }
}
