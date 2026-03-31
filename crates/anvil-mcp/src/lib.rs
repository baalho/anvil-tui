//! MCP (Model Context Protocol) client for Anvil.
//!
//! Connects to external tool servers via the MCP protocol (JSON-RPC over stdio).
//! Each server is spawned as a child process and discovered tools are namespaced
//! as `mcp_{server}_{tool}` to avoid conflicts with built-in tools.
//!
//! # Key types
//! - [`McpManager`] — manages multiple server connections and tool dispatch
//! - [`McpServerConfig`] — configuration for a single MCP server
//! - [`McpTool`] — a discovered tool from an MCP server

pub mod config;
pub mod manager;

pub use config::McpServerConfig;
pub use manager::{McpManager, McpTool};
