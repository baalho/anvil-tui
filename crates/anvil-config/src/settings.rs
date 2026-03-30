//! Top-level settings loaded from `.anvil/config.toml`.
//!
//! Settings are organized into three sections:
//! - `[provider]` — which backend to connect to and which model to use
//! - `[agent]` — context window, loop detection, token limits
//! - `[tools]` — shell timeout, output truncation limits

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
    /// Replace the entire system prompt. Use `.anvil/context.md` for additions instead.
    #[serde(default)]
    pub system_prompt_override: Option<String>,
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
            system_prompt_override: None,
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
