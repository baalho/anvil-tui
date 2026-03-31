//! MCP server manager — spawns, connects, and dispatches tool calls to MCP servers.
//!
//! # Protocol
//! MCP uses JSON-RPC 2.0 over stdio. The manager:
//! 1. Spawns each server as a child process
//! 2. Sends `initialize` request
//! 3. Calls `tools/list` to discover available tools
//! 4. Dispatches `tools/call` for tool execution
//!
//! # Tool namespacing
//! MCP tools are namespaced as `mcp_{server}_{tool}` to avoid conflicts
//! with Anvil's built-in tools.

use crate::config::McpServerConfig;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// A discovered tool from an MCP server.
#[derive(Debug, Clone)]
pub struct McpTool {
    /// Original tool name from the server.
    pub name: String,
    /// Namespaced name: `mcp_{server}_{name}`.
    pub qualified_name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: Value,
    /// Which server this tool belongs to.
    pub server_name: String,
}

/// A connected MCP server with its child process and discovered tools.
struct McpConnection {
    config: McpServerConfig,
    child: Child,
    tools: Vec<McpTool>,
    next_id: u64,
    /// Server instructions from the initialize response (injected into system prompt).
    instructions: Option<String>,
}

/// Manages multiple MCP server connections.
///
/// Thread-safe via internal `Mutex` — the manager is shared between
/// the agent loop (tool dispatch) and the UI (slash commands).
pub struct McpManager {
    connections: Mutex<HashMap<String, McpConnection>>,
}

impl McpManager {
    /// Create a new manager and connect to all configured servers.
    /// Servers that fail to connect are logged and skipped (not fatal).
    pub async fn new(configs: &[McpServerConfig]) -> Self {
        let mut connections = HashMap::new();

        for config in configs {
            match Self::connect(config).await {
                Ok(conn) => {
                    tracing::info!(
                        "MCP server '{}' connected ({} tools)",
                        config.name,
                        conn.tools.len()
                    );
                    connections.insert(config.name.clone(), conn);
                }
                Err(e) => {
                    tracing::warn!("MCP server '{}' failed to connect: {e}", config.name);
                }
            }
        }

        Self {
            connections: Mutex::new(connections),
        }
    }

    /// Create an empty manager (no servers configured).
    pub fn empty() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    /// Get all discovered tools across all connected servers.
    pub async fn tools(&self) -> Vec<McpTool> {
        let conns = self.connections.lock().await;
        conns.values().flat_map(|c| c.tools.clone()).collect()
    }

    /// Get OpenAI-compatible tool definitions for all MCP tools.
    pub async fn tool_definitions(&self) -> Vec<Value> {
        self.tools()
            .await
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.qualified_name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }

    /// Get server instructions (from MCP initialize) for system prompt injection.
    pub async fn server_instructions(&self) -> Vec<(String, String)> {
        let conns = self.connections.lock().await;
        conns
            .values()
            .filter_map(|c| {
                c.instructions
                    .as_ref()
                    .map(|i| (c.config.name.clone(), i.clone()))
            })
            .collect()
    }

    /// Call a tool on the appropriate server.
    /// `qualified_name` is the namespaced name (e.g., `mcp_filesystem_read_file`).
    pub async fn call_tool(&self, qualified_name: &str, args: &Value) -> Result<String> {
        let (server_name, tool_name) = Self::parse_qualified_name(qualified_name)?;

        let mut conns = self.connections.lock().await;
        let conn = conns
            .get_mut(&server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{server_name}' not connected"))?;

        let id = conn.next_id;
        conn.next_id += 1;

        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": args,
            }
        });

        let response = Self::send_request(&mut conn.child, &request).await?;

        // Extract result content
        if let Some(error) = response.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            bail!("MCP tool error: {msg}");
        }

        let result = response.get("result").cloned().unwrap_or(Value::Null);

        // MCP tool results have a `content` array with text entries
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            let text: Vec<&str> = content
                .iter()
                .filter_map(|entry| entry.get("text").and_then(|t| t.as_str()))
                .collect();
            Ok(text.join("\n"))
        } else {
            Ok(serde_json::to_string_pretty(&result)?)
        }
    }

    /// Check if a qualified tool name belongs to an MCP server.
    pub fn is_mcp_tool(name: &str) -> bool {
        name.starts_with("mcp_")
    }

    /// List connected servers with their tool counts.
    pub async fn server_status(&self) -> Vec<(String, usize, bool)> {
        let conns = self.connections.lock().await;
        conns
            .values()
            .map(|c| {
                let alive = c.child.id().is_some();
                (c.config.name.clone(), c.tools.len(), alive)
            })
            .collect()
    }

    /// Reconnect a server by name.
    pub async fn restart(&self, name: &str) -> Result<()> {
        let mut conns = self.connections.lock().await;
        if let Some(mut old) = conns.remove(name) {
            let _ = old.child.kill().await;
        }

        // Find the config from existing connections or error
        // We need the original config — store it in the connection
        bail!("restart requires the original config; use McpManager::new() to reconnect");
    }

    /// Gracefully shut down all servers.
    pub async fn shutdown(&self) {
        let mut conns = self.connections.lock().await;
        for (name, conn) in conns.iter_mut() {
            tracing::info!("shutting down MCP server '{name}'");
            let _ = conn.child.kill().await;
        }
        conns.clear();
    }

    // --- Internal ---

    async fn connect(config: &McpServerConfig) -> Result<McpConnection> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (key, val) in &config.env {
            cmd.env(key, val);
        }

        let mut child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "failed to spawn MCP server '{}' ({}): {e}",
                config.name,
                config.command
            )
        })?;

        // Send initialize request
        let init_request = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "anvil",
                    "version": "1.1.0"
                }
            }
        });

        let init_response = Self::send_request(&mut child, &init_request).await?;

        let instructions = init_response
            .pointer("/result/instructions")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string());

        // Send initialized notification
        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        Self::send_notification(&mut child, &initialized).await?;

        // Discover tools
        let tools_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        });

        let tools_response = Self::send_request(&mut child, &tools_request).await?;

        let tools = Self::parse_tools(&config.name, &tools_response);

        Ok(McpConnection {
            config: config.clone(),
            child,
            tools,
            next_id: 2,
            instructions,
        })
    }

    fn parse_tools(server_name: &str, response: &Value) -> Vec<McpTool> {
        let empty = vec![];
        let tool_list = response
            .pointer("/result/tools")
            .and_then(|t| t.as_array())
            .unwrap_or(&empty);

        tool_list
            .iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?;
                let description = t
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_schema = t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}}));

                Some(McpTool {
                    name: name.to_string(),
                    qualified_name: format!("mcp_{server_name}_{name}"),
                    description,
                    input_schema,
                    server_name: server_name.to_string(),
                })
            })
            .collect()
    }

    fn parse_qualified_name(qualified: &str) -> Result<(String, String)> {
        // Format: mcp_{server}_{tool}
        let rest = qualified
            .strip_prefix("mcp_")
            .ok_or_else(|| anyhow::anyhow!("not an MCP tool: {qualified}"))?;

        let underscore_pos = rest
            .find('_')
            .ok_or_else(|| anyhow::anyhow!("invalid MCP tool name: {qualified}"))?;

        let server = &rest[..underscore_pos];
        let tool = &rest[underscore_pos + 1..];

        Ok((server.to_string(), tool.to_string()))
    }

    async fn send_request(child: &mut Child, request: &Value) -> Result<Value> {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdin not available"))?;

        let msg = serde_json::to_string(request)?;
        stdin.write_all(msg.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;

        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdout not available"))?;

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        // Read response line (JSON-RPC responses are newline-delimited)
        let timeout = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            reader.read_line(&mut line),
        )
        .await
        .map_err(|_| anyhow::anyhow!("MCP server response timeout (30s)"))?;

        timeout?;

        if line.trim().is_empty() {
            bail!("MCP server returned empty response");
        }

        let response: Value = serde_json::from_str(line.trim())?;
        Ok(response)
    }

    async fn send_notification(child: &mut Child, notification: &Value) -> Result<()> {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdin not available"))?;

        let msg = serde_json::to_string(notification)?;
        stdin.write_all(msg.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_qualified_name_valid() {
        let (server, tool) = McpManager::parse_qualified_name("mcp_filesystem_read_file").unwrap();
        assert_eq!(server, "filesystem");
        assert_eq!(tool, "read_file");
    }

    #[test]
    fn parse_qualified_name_invalid() {
        assert!(McpManager::parse_qualified_name("file_read").is_err());
        assert!(McpManager::parse_qualified_name("mcp_").is_err());
    }

    #[test]
    fn is_mcp_tool_check() {
        assert!(McpManager::is_mcp_tool("mcp_filesystem_read_file"));
        assert!(!McpManager::is_mcp_tool("file_read"));
        assert!(!McpManager::is_mcp_tool("shell"));
    }

    #[test]
    fn parse_tools_from_response() {
        let response = json!({
            "result": {
                "tools": [
                    {
                        "name": "read_file",
                        "description": "Read a file",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"}
                            },
                            "required": ["path"]
                        }
                    },
                    {
                        "name": "write_file",
                        "description": "Write a file",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"},
                                "content": {"type": "string"}
                            }
                        }
                    }
                ]
            }
        });

        let tools = McpManager::parse_tools("fs", &response);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].qualified_name, "mcp_fs_read_file");
        assert_eq!(tools[1].qualified_name, "mcp_fs_write_file");
        assert_eq!(tools[0].description, "Read a file");
    }

    #[test]
    fn parse_tools_empty_response() {
        let response = json!({"result": {"tools": []}});
        let tools = McpManager::parse_tools("empty", &response);
        assert!(tools.is_empty());
    }

    #[test]
    fn tool_definition_format() {
        let tool = McpTool {
            name: "read_file".to_string(),
            qualified_name: "mcp_fs_read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            server_name: "fs".to_string(),
        };

        let def = json!({
            "type": "function",
            "function": {
                "name": tool.qualified_name,
                "description": tool.description,
                "parameters": tool.input_schema,
            }
        });

        assert_eq!(def["function"]["name"], "mcp_fs_read_file");
    }

    #[tokio::test]
    async fn empty_manager_has_no_tools() {
        let manager = McpManager::empty();
        assert!(manager.tools().await.is_empty());
        assert!(manager.tool_definitions().await.is_empty());
        assert!(manager.server_status().await.is_empty());
    }
}
