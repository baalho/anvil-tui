//! Model routing — direct specific tool calls to different models.
//!
//! # Why this exists
//! Small models (7-8B) handle simple tasks like shell commands well, while
//! larger models are better for complex reasoning. Routing lets users assign
//! specific tools or skill categories to cheaper/faster models.
//!
//! # How it works
//! Routes are stored as a map from tool name (or `*` for default) to model name.
//! When the agent prepares a request after tool calls, it checks if any of the
//! pending tool results match a route and temporarily switches the model.

use std::collections::HashMap;

/// Routes tool calls to specific models.
#[derive(Debug, Clone, Default)]
pub struct ModelRouter {
    /// Map from tool name to model name. `*` is the default/fallback.
    routes: HashMap<String, String>,
}

impl ModelRouter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a route: tool calls to `tool_name` will use `model_name`.
    pub fn add_route(&mut self, tool_name: &str, model_name: &str) {
        self.routes
            .insert(tool_name.to_string(), model_name.to_string());
    }

    /// Remove a route for a tool.
    pub fn remove_route(&mut self, tool_name: &str) -> bool {
        self.routes.remove(tool_name).is_some()
    }

    /// Get the model to use for a given tool name.
    /// Returns `None` if no route is configured (use default model).
    pub fn model_for_tool(&self, tool_name: &str) -> Option<&str> {
        self.routes
            .get(tool_name)
            .or_else(|| self.routes.get("*"))
            .map(|s| s.as_str())
    }

    /// List all configured routes.
    pub fn routes(&self) -> &HashMap<String, String> {
        &self.routes
    }

    /// Whether any routes are configured.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_routes_returns_none() {
        let router = ModelRouter::new();
        assert!(router.model_for_tool("shell").is_none());
    }

    #[test]
    fn specific_route() {
        let mut router = ModelRouter::new();
        router.add_route("shell", "qwen3:8b");
        assert_eq!(router.model_for_tool("shell"), Some("qwen3:8b"));
        assert!(router.model_for_tool("file_read").is_none());
    }

    #[test]
    fn wildcard_fallback() {
        let mut router = ModelRouter::new();
        router.add_route("*", "qwen3:30b");
        assert_eq!(router.model_for_tool("shell"), Some("qwen3:30b"));
        assert_eq!(router.model_for_tool("file_read"), Some("qwen3:30b"));
    }

    #[test]
    fn specific_overrides_wildcard() {
        let mut router = ModelRouter::new();
        router.add_route("*", "qwen3:30b");
        router.add_route("shell", "qwen3:8b");
        assert_eq!(router.model_for_tool("shell"), Some("qwen3:8b"));
        assert_eq!(router.model_for_tool("file_read"), Some("qwen3:30b"));
    }

    #[test]
    fn remove_route() {
        let mut router = ModelRouter::new();
        router.add_route("shell", "qwen3:8b");
        assert!(router.remove_route("shell"));
        assert!(router.model_for_tool("shell").is_none());
        assert!(!router.remove_route("shell")); // already removed
    }
}
