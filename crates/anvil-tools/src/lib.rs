//! Tool definitions, execution, and permission management for Anvil.
//!
//! This crate provides the 11 built-in tools that the LLM can call:
//! - `shell` — execute shell commands (via `sh -c` / `cmd.exe /C`)
//! - `file_read` — read file contents
//! - `file_write` — create or overwrite files
//! - `file_edit` — search-and-replace within files
//! - `grep` — search file contents with regex
//! - `ls` — list directory contents with metadata
//! - `find` — recursive file search with filtering
//!
//! # Security model
//! - All file operations are sandboxed to the workspace directory
//! - Shell commands use `env_clear()` with explicit safe-var passthrough
//! - Active skills can declare additional env vars for passthrough
//! - Output is tail-truncated to prevent context window overflow

mod definitions;
mod executor;
pub mod hooks;
mod permission;
pub mod plugins;
mod tools;
mod truncation;

pub use definitions::all_tool_definitions;
pub use executor::{KidsSandbox, ToolExecutor, DEFAULT_KIDS_COMMANDS};
pub use hooks::HookRunner;
pub use permission::{PermissionDecision, PermissionHandler};
pub use plugins::{load_plugins, ToolPlugin};
pub use truncation::{TruncationConfig, TruncationResult};

/// Structured tool output — the multi-port manifold for tool results.
///
/// All existing tools return `Text`. Future STEM tools (kinematic physics,
/// 4-bar linkages) can return `Structured` with geometric data in the
/// `data` field, while still providing a human-readable `text` summary
/// for the LLM conversation history.
#[derive(Debug, Clone)]
pub enum ToolOutput {
    /// Plain text result (all current tools use this).
    Text(String),
    /// Structured result with both human-readable text and machine-readable data.
    /// The `text` field goes into the LLM conversation; `data` is available
    /// to the UI layer for rendering (e.g., SVG paths, coordinate arrays).
    /// `content_type` hints the renderer: "text", "image", "svg", "table".
    Structured {
        text: String,
        data: serde_json::Value,
        /// Hint for the UI renderer. Defaults to "text".
        content_type: String,
    },
}

impl ToolOutput {
    /// Get the human-readable text for the LLM conversation history.
    pub fn text(&self) -> &str {
        match self {
            ToolOutput::Text(s) => s,
            ToolOutput::Structured { text, .. } => text,
        }
    }

    /// Convert into the owned text string, discarding structured data.
    pub fn into_text(self) -> String {
        match self {
            ToolOutput::Text(s) => s,
            ToolOutput::Structured { text, .. } => text,
        }
    }
}

impl std::fmt::Display for ToolOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}

impl PartialEq<str> for ToolOutput {
    fn eq(&self, other: &str) -> bool {
        self.text() == other
    }
}

impl PartialEq<&str> for ToolOutput {
    fn eq(&self, other: &&str) -> bool {
        self.text() == *other
    }
}

impl From<String> for ToolOutput {
    fn from(s: String) -> Self {
        ToolOutput::Text(s)
    }
}

impl std::ops::Deref for ToolOutput {
    type Target = str;
    fn deref(&self) -> &str {
        self.text()
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn text_variant_returns_content() {
        let output = ToolOutput::Text("hello".to_string());
        assert_eq!(output.text(), "hello");
    }

    #[test]
    fn structured_variant_returns_text() {
        let output = ToolOutput::Structured {
            text: "summary".to_string(),
            data: serde_json::json!({"x": 1, "y": 2}),
            content_type: "text".to_string(),
        };
        assert_eq!(output.text(), "summary");
    }

    #[test]
    fn into_text_consumes() {
        let output = ToolOutput::Structured {
            text: "consumed".to_string(),
            data: serde_json::json!(null),
            content_type: "text".to_string(),
        };
        assert_eq!(output.into_text(), "consumed");
    }

    #[test]
    fn from_string_creates_text_variant() {
        let output: ToolOutput = "test".to_string().into();
        assert!(matches!(output, ToolOutput::Text(_)));
        assert_eq!(output.text(), "test");
    }
}
