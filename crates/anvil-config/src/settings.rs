//! Top-level settings loaded from `.anvil/config.toml`.
//!
//! Settings are organized into sections:
//! - `[provider]` — which backend to connect to and which model to use
//! - `[agent]` — context window, loop detection, token limits
//! - `[tools]` — shell timeout, output truncation limits
//! - `[[profiles]]` — named launch profiles bundling persona + mode + skills + model

use crate::provider::ProviderConfig;
use serde::{Deserialize, Serialize};

/// Top-level configuration, deserialized from `.anvil/config.toml`.
///
/// All fields have sensible defaults (Ollama on localhost, 8K context, 30s shell timeout).
/// Users only need to override what they want to change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// LLM backend connection settings.
    #[serde(default)]
    pub provider: ProviderConfig,
    /// Agent behavior settings (context window, loop detection, token limits).
    #[serde(default)]
    pub agent: AgentSettings,
    /// Tool execution settings (timeouts, output limits).
    #[serde(default)]
    pub tools: ToolSettings,
    /// MCP (Model Context Protocol) server configuration.
    #[serde(default)]
    pub mcp: McpSettings,
    /// Named launch profiles — bundle persona + mode + skills + model into
    /// a single `anvil --profile <name>` flag.
    #[serde(default)]
    pub profiles: Vec<LaunchProfile>,
    /// Long-running harness settings (planner/generator/evaluator).
    #[serde(default)]
    pub harness: HarnessSettings,
}

/// A named launch profile that bundles startup configuration.
///
/// Profiles let users skip manual `/persona`, `/mode`, `/skill`, `/model`
/// commands at startup. `anvil --profile sparkle` applies everything at once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchProfile {
    /// Profile name used with `--profile` flag.
    pub name: String,
    /// Persona to activate (empty string = no persona).
    #[serde(default)]
    pub persona: String,
    /// Mode to set ("coding" or "creative"). Defaults to mode implied by persona.
    #[serde(default)]
    pub mode: String,
    /// Skills to activate by key.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Model to use (overrides provider.model).
    #[serde(default)]
    pub model: String,
    /// Override the backend base URL for this profile.
    ///
    /// # Why this exists
    /// When running two MLX servers (e.g. LFM2 on :8081 for kids, Qwen3 on
    /// :8080 for coding), each launch profile needs to point at the right
    /// server. Without this, all profiles share the global `provider.base_url`
    /// regardless of which model they request.
    ///
    /// Example:
    /// ```toml
    /// [[profiles]]
    /// name = "sparkle"
    /// model = "lfm2"
    /// base_url = "http://localhost:8081/v1"  # kids server
    /// ```
    #[serde(default)]
    pub base_url: String,
}

/// MCP server configuration — connects to external tool servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpSettings {
    /// List of MCP servers to connect to at startup.
    #[serde(default)]
    pub servers: Vec<McpServerEntry>,
}

/// A single MCP server entry in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// Display name for the server.
    pub name: String,
    /// Command to spawn the server process.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables for the server process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Controls agent loop behavior — context management, safety limits, and prompt overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSettings {
    /// Maximum tokens per session before hard stop. Prevents runaway sessions.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    /// Warn user when token usage exceeds this percentage of max_tokens.
    #[serde(default = "default_warn_threshold")]
    pub warn_threshold_pct: u8,
    /// Maximum consecutive identical tool calls before the agent pauses.
    /// Prevents infinite loops where the LLM repeats the same failing command.
    #[serde(default = "default_loop_limit")]
    pub loop_detection_limit: u32,
    /// Model context window size in tokens. Overridden by model profile if one matches.
    #[serde(default = "default_context_window")]
    pub context_window: usize,
    /// Auto-compact when context usage exceeds this percentage (0-100).
    /// Set to 0 to disable auto-compaction.
    #[serde(default = "default_auto_compact_threshold")]
    pub auto_compact_threshold: u8,
    /// Replace the entire system prompt. Use `.anvil/context.md` for additions instead.
    #[serde(default)]
    pub system_prompt_override: Option<String>,
    /// Restricted workspace path for kids personas. When a kids persona is
    /// active and this is set, all tool paths resolve relative to this
    /// directory instead of the general workspace. Like a governor valve
    /// that limits the operating range of a hydraulic cylinder.
    #[serde(default)]
    pub kids_workspace: Option<String>,
    /// Shell command allowlist for kids personas. When a kids persona is
    /// active, only commands whose first word matches this list are allowed.
    /// Defaults to a safe set if not specified.
    #[serde(default)]
    pub kids_allowed_commands: Option<Vec<String>>,
}

/// Controls the long-running multi-agent harness (planner/generator/evaluator).
///
/// The harness decomposes complex tasks into sprints, implements each with a
/// fresh agent (context reset), and evaluates the result with a separate critic
/// agent. Designed for 64GB local LLM setups where context is scarce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessSettings {
    /// Maximum number of sprints the planner can create.
    #[serde(default = "default_harness_max_sprints")]
    pub max_sprints: usize,
    /// Maximum retry attempts per sprint when the evaluator fails it.
    #[serde(default = "default_harness_max_retries")]
    pub max_retries_per_sprint: usize,
    /// Total token budget across all harness phases.
    #[serde(default = "default_harness_max_tokens")]
    pub max_total_tokens: u64,
    /// Wall-clock timeout in minutes for the entire harness run.
    #[serde(default = "default_harness_max_duration")]
    pub max_duration_minutes: u64,
    /// Model to use for the planner agent. Empty = use default model.
    #[serde(default)]
    pub planner_model: String,
    /// Model to use for the evaluator agent. Empty = use default model.
    /// Can be a smaller/faster model since the evaluator only reads and judges.
    #[serde(default)]
    pub evaluator_model: String,
    /// Maximum turns per generator sprint before forcing completion.
    #[serde(default = "default_harness_sprint_turn_limit")]
    pub sprint_turn_limit: usize,
}

impl Default for HarnessSettings {
    fn default() -> Self {
        Self {
            max_sprints: default_harness_max_sprints(),
            max_retries_per_sprint: default_harness_max_retries(),
            max_total_tokens: default_harness_max_tokens(),
            max_duration_minutes: default_harness_max_duration(),
            planner_model: String::new(),
            evaluator_model: String::new(),
            sprint_turn_limit: default_harness_sprint_turn_limit(),
        }
    }
}

fn default_harness_max_sprints() -> usize {
    10
}
fn default_harness_max_retries() -> usize {
    3
}
fn default_harness_max_tokens() -> u64 {
    500_000
}
fn default_harness_max_duration() -> u64 {
    120
}
fn default_harness_sprint_turn_limit() -> usize {
    20
}

/// Controls tool execution behavior — timeouts and output size limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSettings {
    /// Default timeout for shell commands in seconds. Per-call override via tool args.
    #[serde(default = "default_shell_timeout")]
    pub shell_timeout_secs: u64,
    /// Default timeout for file operations in seconds.
    #[serde(default = "default_file_timeout")]
    pub file_timeout_secs: u64,
    /// Maximum bytes in tool output before tail-truncation kicks in.
    /// Full output is saved to a temp file when truncated.
    #[serde(default = "default_output_limit")]
    pub output_limit: usize,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            warn_threshold_pct: default_warn_threshold(),
            loop_detection_limit: default_loop_limit(),
            context_window: default_context_window(),
            auto_compact_threshold: default_auto_compact_threshold(),
            system_prompt_override: None,
            kids_workspace: None,
            kids_allowed_commands: None,
        }
    }
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            shell_timeout_secs: default_shell_timeout(),
            file_timeout_secs: default_file_timeout(),
            output_limit: default_output_limit(),
        }
    }
}

fn default_max_tokens() -> u64 {
    200_000
}
fn default_warn_threshold() -> u8 {
    80
}
fn default_loop_limit() -> u32 {
    10
}
fn default_context_window() -> usize {
    8192
}
fn default_shell_timeout() -> u64 {
    30
}
fn default_file_timeout() -> u64 {
    5
}
fn default_output_limit() -> usize {
    10_000
}
fn default_auto_compact_threshold() -> u8 {
    90
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profile_with_base_url() {
        let toml_str = r#"
            [[profiles]]
            name = "sparkle"
            persona = "sparkle"
            mode = "creative"
            skills = ["kids-first"]
            model = "lfm2"
            base_url = "http://localhost:8081/v1"

            [[profiles]]
            name = "code"
            mode = "coding"
            model = "qwen3-coder-next"
        "#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.profiles.len(), 2);
        assert_eq!(settings.profiles[0].base_url, "http://localhost:8081/v1");
        // base_url is optional — defaults to empty
        assert!(settings.profiles[1].base_url.is_empty());
    }

    #[test]
    fn parse_settings_with_profiles() {
        let toml_str = r#"
            [[profiles]]
            name = "sparkle"
            persona = "sparkle"
            mode = "creative"
            skills = ["cool-stuff", "story-mode"]
            model = "qwen3:30b"

            [[profiles]]
            name = "code"
            mode = "coding"
            model = "qwen3-coder:30b"
        "#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.profiles.len(), 2);
        assert_eq!(settings.profiles[0].name, "sparkle");
        assert_eq!(settings.profiles[0].persona, "sparkle");
        assert_eq!(settings.profiles[0].mode, "creative");
        assert_eq!(
            settings.profiles[0].skills,
            vec!["cool-stuff", "story-mode"]
        );
        assert_eq!(settings.profiles[1].name, "code");
        assert!(settings.profiles[1].persona.is_empty());
    }

    #[test]
    fn empty_profiles_is_default() {
        let settings = Settings::default();
        assert!(settings.profiles.is_empty());
    }

    #[test]
    fn settings_without_profiles_parses() {
        let toml_str = r#"
            [provider]
            model = "test-model"
        "#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert!(settings.profiles.is_empty());
        assert_eq!(settings.provider.model, "test-model");
    }
}
