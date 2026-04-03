//! Host inventory — parsed from `.anvil/inventory.toml`.
//!
//! Provides the LLM with awareness of which hosts exist, how to reach them,
//! and what services run where. Injected into the system prompt so the agent
//! can route commands like "start the Valheim server" to the correct host.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Top-level inventory structure.
///
/// Parsed from `.anvil/inventory.toml`. Contains a list of hosts with their
/// Tailscale hostnames, roles, container runtimes, and deployed services.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Inventory {
    /// List of managed hosts.
    #[serde(default)]
    pub hosts: Vec<Host>,
}

/// A single host in the inventory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    /// Short identifier (e.g., "debian-1").
    pub name: String,
    /// Tailscale MagicDNS hostname for SSH access.
    pub tailscale_name: String,
    /// SSH username.
    pub user: String,
    /// Operating system: "macos", "linux", or "windows".
    pub os: String,
    /// Role: "workstation", "server", or "dev".
    pub role: String,
    /// Container runtime: "docker" or "podman".
    pub container_runtime: String,
    /// Services deployed on this host (simple string list, backward compatible).
    #[serde(default)]
    pub services: Vec<String>,
    /// Structured service definitions with ports and secrets (optional).
    /// When present, these are used for deployment context instead of `services`.
    #[serde(default)]
    pub deployments: Vec<Deployment>,
}

/// A structured service deployment on a host.
///
/// Extends the simple `services` string list with port and secrets info
/// for deployment workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    /// Service name (e.g., "valheim", "caddy").
    pub name: String,
    /// Port the service listens on (optional).
    pub port: Option<u16>,
    /// Path to SOPS-encrypted secrets file (optional).
    /// Relative to workspace root (e.g., "secrets/valheim.env").
    pub secrets: Option<String>,
    /// Compose file path relative to deployment root (optional).
    pub compose_file: Option<String>,
}

/// Load inventory from `.anvil/inventory.toml`.
///
/// Returns `Inventory::default()` (empty hosts) if the file doesn't exist
/// or can't be parsed. Never fails — inventory is optional.
pub fn load_inventory(workspace: &Path) -> Inventory {
    let path = workspace.join(".anvil/inventory.toml");
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => Inventory::default(),
    }
}

/// Format inventory as a markdown section for the system prompt.
///
/// Returns `None` if the inventory has no hosts.
pub fn inventory_as_prompt(inventory: &Inventory) -> Option<String> {
    if inventory.hosts.is_empty() {
        return None;
    }

    let mut out = String::from("## Infrastructure Inventory\n\n");
    out.push_str("| Host | Tailscale | User | OS | Runtime | Services |\n");
    out.push_str("|------|-----------|------|----|---------|----------|\n");

    for h in &inventory.hosts {
        let services = if h.services.is_empty() {
            "—".to_string()
        } else {
            h.services.join(", ")
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            h.name, h.tailscale_name, h.user, h.os, h.container_runtime, services
        ));
    }

    out.push_str("\nTo run commands on a remote host:\n");
    out.push_str("  ssh <user>@<tailscale_name> '<command>'\n");

    // Deployment details (if any host has structured deployments)
    let has_deployments = inventory.hosts.iter().any(|h| !h.deployments.is_empty());
    if has_deployments {
        out.push_str("\n### Deployments\n\n");
        for h in &inventory.hosts {
            for d in &h.deployments {
                out.push_str(&format!("- **{}** on {} ({})\n", d.name, h.name, h.container_runtime));
                if let Some(port) = d.port {
                    out.push_str(&format!("  - Port: {port}\n"));
                }
                if let Some(ref secrets) = d.secrets {
                    out.push_str(&format!("  - Secrets: `{secrets}` (SOPS-encrypted)\n"));
                }
                if let Some(ref compose) = d.compose_file {
                    out.push_str(&format!("  - Compose: `{compose}`\n"));
                }
                out.push_str(&format!(
                    "  - Deploy: `ssh {}@{} '{} compose up -d'`\n",
                    h.user, h.tailscale_name, h.container_runtime
                ));
            }
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_valid_inventory() {
        let toml = r#"
[[hosts]]
name = "macbook"
tailscale_name = "macbook-pro"
user = "baalho"
os = "macos"
role = "workstation"
container_runtime = "docker"
services = ["immich", "paperless"]

[[hosts]]
name = "debian-1"
tailscale_name = "debian-server-1"
user = "deploy"
os = "linux"
role = "server"
container_runtime = "podman"
services = ["valheim", "caddy"]
"#;
        let inv: Inventory = toml::from_str(toml).unwrap();
        assert_eq!(inv.hosts.len(), 2);
        assert_eq!(inv.hosts[0].name, "macbook");
        assert_eq!(inv.hosts[0].container_runtime, "docker");
        assert_eq!(inv.hosts[1].services, vec!["valheim", "caddy"]);
    }

    #[test]
    fn parse_empty_hosts() {
        let toml = "hosts = []\n";
        let inv: Inventory = toml::from_str(toml).unwrap();
        assert!(inv.hosts.is_empty());
    }

    #[test]
    fn parse_empty_file() {
        let inv: Inventory = toml::from_str("").unwrap();
        assert!(inv.hosts.is_empty());
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = TempDir::new().unwrap();
        let inv = load_inventory(dir.path());
        assert!(inv.hosts.is_empty());
    }

    #[test]
    fn load_valid_file() {
        let dir = TempDir::new().unwrap();
        let anvil_dir = dir.path().join(".anvil");
        std::fs::create_dir_all(&anvil_dir).unwrap();
        std::fs::write(
            anvil_dir.join("inventory.toml"),
            r#"
[[hosts]]
name = "test"
tailscale_name = "test-host"
user = "admin"
os = "linux"
role = "server"
container_runtime = "podman"
services = ["nginx"]
"#,
        )
        .unwrap();

        let inv = load_inventory(dir.path());
        assert_eq!(inv.hosts.len(), 1);
        assert_eq!(inv.hosts[0].name, "test");
    }

    #[test]
    fn prompt_empty_inventory_returns_none() {
        let inv = Inventory::default();
        assert!(inventory_as_prompt(&inv).is_none());
    }

    #[test]
    fn prompt_with_hosts_returns_table() {
        let inv = Inventory {
            hosts: vec![Host {
                name: "srv".to_string(),
                tailscale_name: "srv-ts".to_string(),
                user: "deploy".to_string(),
                os: "linux".to_string(),
                role: "server".to_string(),
                container_runtime: "podman".to_string(),
                services: vec!["web".to_string(), "db".to_string()],
                deployments: vec![],
            }],
        };
        let prompt = inventory_as_prompt(&inv).unwrap();
        assert!(prompt.contains("## Infrastructure Inventory"));
        assert!(prompt.contains("| srv | srv-ts | deploy | linux | podman | web, db |"));
        assert!(prompt.contains("ssh <user>@<tailscale_name>"));
    }

    #[test]
    fn prompt_host_with_no_services() {
        let inv = Inventory {
            hosts: vec![Host {
                name: "dev".to_string(),
                tailscale_name: "dev-ts".to_string(),
                user: "me".to_string(),
                os: "macos".to_string(),
                role: "workstation".to_string(),
                container_runtime: "docker".to_string(),
                services: vec![],
                deployments: vec![],
            }],
        };
        let prompt = inventory_as_prompt(&inv).unwrap();
        assert!(prompt.contains("| dev |"));
        assert!(prompt.contains("| — |"));
    }

    #[test]
    fn parse_deployments() {
        let toml = r#"
[[hosts]]
name = "srv"
tailscale_name = "srv-ts"
user = "deploy"
os = "linux"
role = "server"
container_runtime = "podman"

[[hosts.deployments]]
name = "valheim"
port = 2456
secrets = "secrets/valheim.env"
compose_file = "docker-compose.yml"

[[hosts.deployments]]
name = "caddy"
port = 443
"#;
        let inv: Inventory = toml::from_str(toml).unwrap();
        assert_eq!(inv.hosts[0].deployments.len(), 2);
        assert_eq!(inv.hosts[0].deployments[0].name, "valheim");
        assert_eq!(inv.hosts[0].deployments[0].port, Some(2456));
        assert_eq!(
            inv.hosts[0].deployments[0].secrets,
            Some("secrets/valheim.env".to_string())
        );
        assert_eq!(inv.hosts[0].deployments[1].name, "caddy");
        assert!(inv.hosts[0].deployments[1].secrets.is_none());
    }

    #[test]
    fn prompt_includes_deployment_details() {
        let inv = Inventory {
            hosts: vec![Host {
                name: "srv".to_string(),
                tailscale_name: "srv-ts".to_string(),
                user: "deploy".to_string(),
                os: "linux".to_string(),
                role: "server".to_string(),
                container_runtime: "podman".to_string(),
                services: vec![],
                deployments: vec![Deployment {
                    name: "valheim".to_string(),
                    port: Some(2456),
                    secrets: Some("secrets/valheim.env".to_string()),
                    compose_file: None,
                }],
            }],
        };
        let prompt = inventory_as_prompt(&inv).unwrap();
        assert!(prompt.contains("### Deployments"));
        assert!(prompt.contains("**valheim** on srv"));
        assert!(prompt.contains("Port: 2456"));
        assert!(prompt.contains("secrets/valheim.env"));
        assert!(prompt.contains("SOPS-encrypted"));
    }

    #[test]
    fn deployments_default_to_empty() {
        let toml = r#"
[[hosts]]
name = "test"
tailscale_name = "test-ts"
user = "admin"
os = "linux"
role = "server"
container_runtime = "docker"
services = ["nginx"]
"#;
        let inv: Inventory = toml::from_str(toml).unwrap();
        assert!(inv.hosts[0].deployments.is_empty());
    }
}
