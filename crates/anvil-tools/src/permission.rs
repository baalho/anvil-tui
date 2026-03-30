//! Permission management for tool execution.
//!
//! # Permission model
//! - Read-only tools (`file_read`, `grep`) execute without asking
//! - Mutating tools (`shell`, `file_write`, `file_edit`) require user approval
//! - Users can grant "always allow" per tool for the session duration
//! - Autonomous mode (`--autonomous`) auto-approves everything

use std::collections::HashSet;
use std::sync::Mutex;

/// The user's response to a tool permission prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Allow this single tool call.
    Allow,
    /// Deny this tool call.
    Deny,
    /// Allow this tool for the rest of the session.
    AllowAlways,
}

/// Tracks which tools have been granted "always allow" for this session.
///
/// Uses `Mutex` because permission checks happen from the agent loop
/// while grants come from the UI thread (interactive mode).
pub struct PermissionHandler {
    always_allowed: Mutex<HashSet<String>>,
}

impl PermissionHandler {
    pub fn new() -> Self {
        Self {
            always_allowed: Mutex::new(HashSet::new()),
        }
    }

    /// Check if a tool has been granted permanent permission for this session.
    pub fn is_always_allowed(&self, tool_name: &str) -> bool {
        self.always_allowed.lock().unwrap().contains(tool_name)
    }

    /// Grant permanent permission for a tool (lasts until session ends).
    pub fn grant_always(&self, tool_name: &str) {
        self.always_allowed
            .lock()
            .unwrap()
            .insert(tool_name.to_string());
    }

    /// Check if a tool is read-only (doesn't need permission).
    pub fn is_read_only(tool_name: &str) -> bool {
        matches!(tool_name, "file_read" | "grep")
    }
}

impl Default for PermissionHandler {
    fn default() -> Self {
        Self::new()
    }
}
