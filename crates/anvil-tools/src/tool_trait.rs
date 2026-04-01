//! Universal tool trait — the expansion port for all tool types.
//!
//! # Why this exists
//! Anvil needs a single interface for built-in tools (shell, file_read),
//! MCP server tools, TOML plugin tools, and future STEM visualization tools.
//! This trait defines how a tool declares its JSON schema to the LLM and
//! how it returns structured results.
//!
//! # Result types
//! Tools return `ToolOutput` which can carry text, structured data (JSON),
//! or binary payloads (for future rendering). The core loop handles all
//! variants without knowing the tool's internals.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// The result of a tool execution.
///
/// Supports text output (most tools), structured JSON (for tools that
/// return data the LLM should reason about), and binary payloads
/// (for future STEM visualizations that render in the TUI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolOutput {
    /// Plain text output (shell stdout, file contents, etc.)
    Text(String),

    /// Structured JSON data. The LLM receives the JSON as a string,
    /// but the TUI can inspect the `kind` field for special rendering.
    /// Example: a physics tool returns `kind: "geometry"` with vertex data.
    Structured {
        /// Machine-readable type tag (e.g., "geometry", "table", "chart").
        kind: String,
        /// The data payload.
        data: Value,
        /// Human-readable summary for the LLM.
        summary: String,
    },

    /// An error message to feed back to the LLM.
    Error(String),
}

/// Well-known structured output kinds for STEM tools.
/// TUI renderers can match on these to provide visual output.
pub mod stem_kinds {
    /// 2D/3D geometry data (vertices, edges, polygons).
    pub const GEOMETRY: &str = "geometry";
    /// Tabular data (rows and columns).
    pub const TABLE: &str = "table";
    /// Chart data (x/y series, labels).
    pub const CHART: &str = "chart";
    /// Physics simulation state (positions, velocities, forces).
    pub const PHYSICS: &str = "physics";
    /// Mathematical expression or equation.
    pub const MATH: &str = "math";
}

impl ToolOutput {
    /// Create a structured output with a well-known STEM kind.
    pub fn stem(kind: &str, data: Value, summary: impl Into<String>) -> Self {
        ToolOutput::Structured {
            kind: kind.to_string(),
            data,
            summary: summary.into(),
        }
    }

    /// Convert to a string suitable for the LLM's tool result message.
    pub fn to_llm_string(&self) -> String {
        match self {
            ToolOutput::Text(s) => s.clone(),
            ToolOutput::Structured {
                kind,
                data,
                summary,
            } => {
                format!(
                    "[{kind}] {summary}\n\n{}",
                    serde_json::to_string_pretty(data).unwrap_or_default()
                )
            }
            ToolOutput::Error(e) => format!("error: {e}"),
        }
    }

    /// Check if this output has a structured kind that the TUI can render.
    pub fn renderable_kind(&self) -> Option<&str> {
        match self {
            ToolOutput::Structured { kind, .. } => Some(kind),
            _ => None,
        }
    }
}

/// Classification of a tool's side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAccess {
    /// Tool only reads data — can run in parallel, no permission needed.
    ReadOnly,
    /// Tool modifies state — runs sequentially, needs user permission.
    Mutating,
}

/// The universal tool trait. All tool types implement this.
///
/// # Implementors
/// - Built-in tools (shell, file_read, git_status, etc.)
/// - MCP server tools (via McpManager adapter)
/// - TOML plugin tools (via PluginTool adapter)
/// - Future STEM tools (physics, geometry, etc.)
#[async_trait::async_trait]
pub trait DynTool: Send + Sync {
    /// Unique name of the tool (e.g., "shell", "mcp_fs_read", "physics_sim").
    fn name(&self) -> &str;

    /// Human-readable description for the LLM.
    fn description(&self) -> &str;

    /// JSON schema for the tool's parameters, in OpenAI function-calling format.
    /// This is sent to the LLM so it knows how to call the tool.
    fn parameter_schema(&self) -> Value;

    /// Whether this tool is read-only or mutating.
    fn access(&self) -> ToolAccess;

    /// Execute the tool with the given arguments.
    ///
    /// `workspace` is the project root directory.
    /// `args` is the JSON object the LLM provided.
    async fn execute(&self, workspace: &Path, args: &Value) -> Result<ToolOutput>;

    /// Build the complete OpenAI function-calling tool definition.
    /// Default implementation assembles name + description + schema.
    fn to_definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameter_schema()
            }
        })
    }
}

/// A registry of tool plugins. Provides lookup by name and merged definitions.
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Box<dyn DynTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Register a tool plugin.
    pub fn register(&mut self, tool: Box<dyn DynTool>) {
        self.tools.push(tool);
    }

    /// Get all tool definitions for the LLM.
    pub fn definitions(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.to_definition()).collect()
    }

    /// Find a tool by name.
    pub fn find(&self, name: &str) -> Option<&dyn DynTool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// Check if a tool is read-only.
    pub fn is_read_only(&self, name: &str) -> bool {
        self.find(name)
            .map(|t| t.access() == ToolAccess::ReadOnly)
            .unwrap_or(false)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTool;

    #[async_trait::async_trait]
    impl DynTool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "A test tool"
        }
        fn parameter_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": ["input"]
            })
        }
        fn access(&self) -> ToolAccess {
            ToolAccess::ReadOnly
        }
        async fn execute(&self, _workspace: &Path, args: &Value) -> Result<ToolOutput> {
            let input = args["input"].as_str().unwrap_or("none");
            Ok(ToolOutput::Text(format!("got: {input}")))
        }
    }

    struct GeometryTool;

    #[async_trait::async_trait]
    impl DynTool for GeometryTool {
        fn name(&self) -> &str {
            "geometry"
        }
        fn description(&self) -> &str {
            "Compute geometry for a 4-bar linkage"
        }
        fn parameter_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "link_lengths": {
                        "type": "array",
                        "items": { "type": "number" }
                    }
                },
                "required": ["link_lengths"]
            })
        }
        fn access(&self) -> ToolAccess {
            ToolAccess::ReadOnly
        }
        async fn execute(&self, _workspace: &Path, args: &Value) -> Result<ToolOutput> {
            let links = args["link_lengths"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_f64()).collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(ToolOutput::Structured {
                kind: "geometry".to_string(),
                data: serde_json::json!({
                    "vertices": [[0.0, 0.0], [links.first().unwrap_or(&1.0), 0.0]],
                    "link_count": links.len()
                }),
                summary: format!("{}-bar linkage computed", links.len()),
            })
        }
    }

    #[test]
    fn stem_helper_creates_structured() {
        let output = ToolOutput::stem(
            stem_kinds::PHYSICS,
            serde_json::json!({"velocity": [1.0, 2.0]}),
            "Ball at t=1s",
        );
        assert_eq!(output.renderable_kind(), Some("physics"));
        assert!(output.to_llm_string().contains("[physics]"));
        assert!(output.to_llm_string().contains("Ball at t=1s"));
    }

    #[test]
    fn stem_kinds_are_distinct() {
        let kinds = [
            stem_kinds::GEOMETRY,
            stem_kinds::TABLE,
            stem_kinds::CHART,
            stem_kinds::PHYSICS,
            stem_kinds::MATH,
        ];
        // All unique
        let mut set = std::collections::HashSet::new();
        for k in &kinds {
            assert!(set.insert(k), "duplicate kind: {k}");
        }
    }

    #[test]
    fn tool_output_text_to_llm() {
        let output = ToolOutput::Text("hello".to_string());
        assert_eq!(output.to_llm_string(), "hello");
        assert!(output.renderable_kind().is_none());
    }

    #[test]
    fn tool_output_structured_to_llm() {
        let output = ToolOutput::Structured {
            kind: "geometry".to_string(),
            data: serde_json::json!({"x": 1}),
            summary: "a shape".to_string(),
        };
        let s = output.to_llm_string();
        assert!(s.contains("[geometry]"));
        assert!(s.contains("a shape"));
        assert_eq!(output.renderable_kind(), Some("geometry"));
    }

    #[test]
    fn tool_output_error() {
        let output = ToolOutput::Error("boom".to_string());
        assert_eq!(output.to_llm_string(), "error: boom");
    }

    #[tokio::test]
    async fn dummy_tool_executes() {
        let tool = DummyTool;
        let result = tool
            .execute(Path::new("/tmp"), &serde_json::json!({"input": "test"}))
            .await
            .unwrap();
        assert!(matches!(result, ToolOutput::Text(s) if s == "got: test"));
    }

    #[tokio::test]
    async fn geometry_tool_returns_structured() {
        let tool = GeometryTool;
        let result = tool
            .execute(
                Path::new("/tmp"),
                &serde_json::json!({"link_lengths": [1.0, 2.0, 1.5, 2.5]}),
            )
            .await
            .unwrap();
        match result {
            ToolOutput::Structured {
                kind,
                data,
                summary,
            } => {
                assert_eq!(kind, "geometry");
                assert_eq!(data["link_count"], 4);
                assert!(summary.contains("4-bar"));
            }
            _ => panic!("expected Structured"),
        }
    }

    #[test]
    fn tool_definition_format() {
        let tool = DummyTool;
        let def = tool.to_definition();
        assert_eq!(def["type"], "function");
        assert_eq!(def["function"]["name"], "dummy");
        assert!(def["function"]["parameters"]["properties"]["input"].is_object());
    }

    #[test]
    fn registry_operations() {
        let mut registry = ToolRegistry::new();
        assert!(registry.is_empty());

        registry.register(Box::new(DummyTool));
        registry.register(Box::new(GeometryTool));

        assert_eq!(registry.len(), 2);
        assert!(registry.find("dummy").is_some());
        assert!(registry.find("geometry").is_some());
        assert!(registry.find("nonexistent").is_none());
        assert!(registry.is_read_only("dummy"));
        assert!(!registry.is_read_only("nonexistent"));
    }

    #[test]
    fn registry_definitions() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(DummyTool));
        registry.register(Box::new(GeometryTool));

        let defs = registry.definitions();
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|d| d["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"dummy"));
        assert!(names.contains(&"geometry"));
    }
}
