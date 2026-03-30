use crate::message::{ApiUsage, ToolCall, ToolCallFunction};
use serde::Deserialize;

/// Events emitted during SSE streaming of a chat completion.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of text content from the assistant.
    ContentDelta(String),
    /// A tool call being assembled (may arrive across multiple chunks).
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    /// Final usage statistics (sent with the last chunk).
    Usage(ApiUsage),
    /// Stream finished.
    Done,
}

/// Raw SSE chunk from the OpenAI-compatible streaming API.
#[derive(Debug, Deserialize)]
pub(crate) struct StreamChunk {
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamChoice {
    pub delta: StreamDelta,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamToolCallDelta {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

/// Accumulates streaming tool call deltas into complete ToolCall objects.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: Vec<PartialToolCall>,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    pub fn push_delta(
        &mut self,
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: &str,
    ) {
        while self.calls.len() <= index {
            self.calls.push(PartialToolCall::default());
        }
        let call = &mut self.calls[index];
        if let Some(id) = id {
            call.id = id;
        }
        if let Some(name) = name {
            call.name = name;
        }
        call.arguments.push_str(arguments_delta);
    }

    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| ToolCall {
                id: c.id,
                call_type: "function".to_string(),
                function: ToolCallFunction {
                    name: c.name,
                    arguments: c.arguments,
                },
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }
}
