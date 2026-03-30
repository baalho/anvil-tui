//! Message types for the OpenAI-compatible chat completions API.
//!
//! These types map directly to the OpenAI API wire format, which is also used by
//! Ollama, llama-server, and mlx_lm.server. The key types are:
//! - `ChatMessage` — a single message in the conversation (system/user/assistant/tool)
//! - `ChatRequest` — the full request body sent to `/v1/chat/completions`
//! - `ChatResponse` — the response body (non-streaming)
//! - `ToolCall` / `ToolDefinition` — function calling protocol

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The role of a message in the conversation.
///
/// Maps to OpenAI's role field. The `Tool` role is used for tool call results —
/// the LLM sends a tool call, we execute it, and send the result back as a
/// `Tool` message with the matching `tool_call_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A single message in the conversation history.
///
/// # Tool call flow
/// 1. User sends a `User` message
/// 2. LLM responds with an `Assistant` message containing `tool_calls`
/// 3. We execute each tool and send a `Tool` message with the result
/// 4. LLM responds to the tool results with another `Assistant` message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// Create a system message (sets the agent's behavior and context).
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create a user message (the human's input).
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create an assistant message (the LLM's response).
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create a tool result message (response to a tool call).
    ///
    /// The `tool_call_id` must match the `id` from the corresponding `ToolCall`
    /// in the assistant's message. This is how the LLM correlates results with requests.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A tool call requested by the LLM.
///
/// The LLM emits these in its response when it wants to use a tool.
/// `arguments` is a JSON string that we parse to extract tool parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFunction,
}

/// The function name and arguments for a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    /// JSON-encoded arguments string. Parsed by the tool executor.
    pub arguments: String,
}

/// The request body for `/v1/chat/completions`.
///
/// # Sampling parameters
/// `temperature`, `top_p`, `min_p`, `repeat_penalty`, and `top_k` are populated
/// from the active model profile (`.anvil/models/*.toml`). When `None`, the
/// backend uses its own defaults.
///
/// # Why optional sampling fields
/// Not all backends support all parameters. Ollama ignores `min_p`.
/// llama-server supports everything. By using `skip_serializing_if = "Option::is_none"`,
/// we only send parameters the user explicitly configured, avoiding backend errors
/// from unsupported fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
    /// Penalizes repeated tokens. Set to 1.0 to disable (required for GLM-4.7).
    /// Named `repeat_penalty` for llama-server; Ollama uses `repeat_penalty` too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub stream: bool,
}

/// The response body from `/v1/chat/completions` (non-streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub usage: Option<ApiUsage>,
}

/// A single completion choice from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

/// Token usage statistics from the API response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// A tool definition sent to the LLM so it knows what tools are available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

/// Describes a function the LLM can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: ToolParameters,
}

/// JSON Schema for a tool's parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameters {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: HashMap<String, ToolParameterProperty>,
    #[serde(default)]
    pub required: Vec<String>,
}

/// A single parameter in a tool's schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameterProperty {
    #[serde(rename = "type")]
    pub prop_type: String,
    pub description: String,
}
