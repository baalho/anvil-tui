//! Tool definitions, execution, and permission management for Anvil.
//!
//! This crate provides the 7 built-in tools that the LLM can call:
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
pub use executor::ToolExecutor;
pub use hooks::HookRunner;
pub use permission::{PermissionDecision, PermissionHandler};
pub use plugins::{load_plugins, ToolPlugin};
pub use truncation::{TruncationConfig, TruncationResult};
