//! OpenAI-compatible LLM client with streaming, retry, and model profile support.
//!
//! This crate handles all communication with LLM backends. It is backend-agnostic —
//! any server that implements the OpenAI chat completions API works (Ollama,
//! llama-server, mlx_lm.server, vLLM, LM Studio, etc.).
//!
//! # Key types
//! - [`LlmClient`] — HTTP client with retry, streaming, and sampling injection
//! - [`ChatRequest`] / [`ChatResponse`] — API wire format types
//! - [`StreamEvent`] — events emitted during SSE streaming
//! - [`TokenUsage`] — cumulative token usage tracking

mod client;
mod message;
pub mod retry;
mod stream;
mod usage;

pub use client::LlmClient;
pub use message::{
    ChatMessage, ChatRequest, ChatResponse, Role, ToolCall, ToolCallFunction, ToolDefinition,
    ToolParameterProperty, ToolParameters,
};
pub use retry::RetryConfig;
pub use stream::{StreamEvent, ToolCallAccumulator};
pub use usage::TokenUsage;
