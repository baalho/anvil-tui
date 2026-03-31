//! MCP server configuration — parsed from `.anvil/config.toml`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a single MCP server.
///
/// # Example config.toml
/// ```toml
/// [[mcp.servers]]
/// name = "filesystem"
/// command = "npx"
/// args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name for the server (used in `/mcp` listing and tool namespacing).
    pub name: String,
    /// Command to spawn the server process.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_server_config() {
        let toml_str = r#"
            name = "filesystem"
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        "#;
        let config: McpServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.name, "filesystem");
        assert_eq!(config.command, "npx");
        assert_eq!(config.args.len(), 3);
    }

    #[test]
    fn parse_server_config_with_env() {
        let toml_str = r#"
            name = "db"
            command = "mcp-server-postgres"
            args = []
            [env]
            DATABASE_URL = "postgres://localhost/mydb"
        "#;
        let config: McpServerConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.env.get("DATABASE_URL").unwrap(),
            "postgres://localhost/mydb"
        );
    }
}
