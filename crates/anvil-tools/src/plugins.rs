//! Custom tool plugins loaded from `.anvil/tools/*.toml`.
//!
//! # How it works
//! Each TOML file defines a tool with a name, description, parameters, and a
//! shell command template. Template variables (`{{arg_name}}`) are substituted
//! with the LLM's arguments before execution.
//!
//! # Example `.anvil/tools/deploy.toml`
//! ```toml
//! name = "deploy"
//! description = "Deploy the application to a target environment"
//!
//! [[params]]
//! name = "environment"
//! type = "string"
//! description = "Target environment (staging, production)"
//! required = true
//!
//! [[params]]
//! name = "dry_run"
//! type = "boolean"
//! description = "Preview changes without applying"
//! required = false
//!
//! [command]
//! template = "deploy.sh --env {{environment}} {{#dry_run}}--dry-run{{/dry_run}}"
//! ```

use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

/// A custom tool defined in `.anvil/tools/*.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolPlugin {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub params: Vec<PluginParam>,
    pub command: PluginCommand,
}

/// A parameter for a custom tool.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginParam {
    pub name: String,
    #[serde(rename = "type", default = "default_param_type")]
    pub param_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

fn default_param_type() -> String {
    "string".to_string()
}

/// The shell command template for a custom tool.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginCommand {
    pub template: String,
}

impl ToolPlugin {
    /// Build the OpenAI function-calling JSON schema for this plugin.
    pub fn to_tool_definition(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for param in &self.params {
            properties.insert(
                param.name.clone(),
                json!({
                    "type": param.param_type,
                    "description": param.description,
                }),
            );
            if param.required {
                required.push(Value::String(param.name.clone()));
            }
        }

        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            }
        })
    }

    /// Render the shell command template with the given arguments.
    /// `{{arg_name}}` is replaced with the string value of the argument.
    /// `{{#bool_arg}}text{{/bool_arg}}` is included only if the bool arg is true.
    pub fn render_command(&self, args: &Value) -> Result<String> {
        let mut cmd = self.template_substitute(&self.command.template, args);

        // Process boolean conditionals: {{#flag}}text{{/flag}}
        loop {
            let start_marker = "{{#";
            let Some(start) = cmd.find(start_marker) else {
                break;
            };
            let rest = &cmd[start + start_marker.len()..];
            let Some(end_name) = rest.find("}}") else {
                break;
            };
            let flag_name = &rest[..end_name];
            let close_tag = format!("{{{{/{flag_name}}}}}");
            let Some(close_pos) = cmd.find(&close_tag) else {
                break;
            };

            let block_start = start + start_marker.len() + end_name + 2; // after {{#name}}
            let block_content = &cmd[block_start..close_pos];
            let block_end = close_pos + close_tag.len();

            let flag_value = args
                .get(flag_name)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let replacement = if flag_value {
                block_content.to_string()
            } else {
                String::new()
            };

            cmd = format!("{}{}{}", &cmd[..start], replacement, &cmd[block_end..]);
        }

        Ok(cmd.trim().to_string())
    }

    /// Simple `{{name}}` substitution.
    fn template_substitute(&self, template: &str, args: &Value) -> String {
        let mut result = template.to_string();
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                let placeholder = format!("{{{{{key}}}}}");
                let replacement = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => value.to_string(),
                };
                result = result.replace(&placeholder, &replacement);
            }
        }
        result
    }
}

/// Load all custom tool plugins from a directory.
pub fn load_plugins(dir: &Path) -> Vec<ToolPlugin> {
    let mut plugins = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return plugins,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<ToolPlugin>(&content) {
                    Ok(plugin) => plugins.push(plugin),
                    Err(e) => {
                        tracing::warn!("invalid tool plugin {}: {e}", path.display());
                    }
                },
                Err(e) => {
                    tracing::warn!("cannot read {}: {e}", path.display());
                }
            }
        }
    }

    plugins
}

/// Validate that a plugin name doesn't conflict with built-in tools.
pub fn validate_plugin_name(name: &str) -> Result<()> {
    const BUILTIN: &[&str] = &[
        "file_read",
        "file_write",
        "file_edit",
        "shell",
        "grep",
        "ls",
        "find",
        "git_status",
        "git_diff",
        "git_log",
        "git_commit",
    ];
    if BUILTIN.contains(&name) {
        bail!("plugin name '{name}' conflicts with built-in tool");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plugin() -> ToolPlugin {
        toml::from_str(
            r#"
            name = "deploy"
            description = "Deploy to environment"

            [[params]]
            name = "environment"
            type = "string"
            description = "Target env"
            required = true

            [[params]]
            name = "dry_run"
            type = "boolean"
            description = "Dry run mode"
            required = false

            [command]
            template = "deploy.sh --env {{environment}} {{#dry_run}}--dry-run{{/dry_run}}"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn parse_plugin_toml() {
        let plugin = sample_plugin();
        assert_eq!(plugin.name, "deploy");
        assert_eq!(plugin.params.len(), 2);
        assert!(plugin.params[0].required);
        assert!(!plugin.params[1].required);
    }

    #[test]
    fn render_command_with_args() {
        let plugin = sample_plugin();
        let args = json!({"environment": "staging", "dry_run": true});
        let cmd = plugin.render_command(&args).unwrap();
        assert_eq!(cmd, "deploy.sh --env staging --dry-run");
    }

    #[test]
    fn render_command_bool_false_omits_block() {
        let plugin = sample_plugin();
        let args = json!({"environment": "production", "dry_run": false});
        let cmd = plugin.render_command(&args).unwrap();
        assert_eq!(cmd, "deploy.sh --env production");
    }

    #[test]
    fn to_tool_definition_schema() {
        let plugin = sample_plugin();
        let def = plugin.to_tool_definition();
        assert_eq!(def["function"]["name"], "deploy");
        let required = def["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "environment");
    }

    #[test]
    fn validate_builtin_name_rejected() {
        assert!(validate_plugin_name("shell").is_err());
        assert!(validate_plugin_name("file_read").is_err());
        assert!(validate_plugin_name("git_status").is_err());
        assert!(validate_plugin_name("git_commit").is_err());
    }

    #[test]
    fn validate_custom_name_accepted() {
        assert!(validate_plugin_name("deploy").is_ok());
        assert!(validate_plugin_name("my_tool").is_ok());
    }

    #[test]
    fn load_plugins_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let plugins = load_plugins(dir.path());
        assert!(plugins.is_empty());
    }

    #[test]
    fn load_plugins_from_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("deploy.toml"),
            r#"
            name = "deploy"
            description = "Deploy"
            [command]
            template = "echo deploy"
            "#,
        )
        .unwrap();
        let plugins = load_plugins(dir.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "deploy");
    }
}
