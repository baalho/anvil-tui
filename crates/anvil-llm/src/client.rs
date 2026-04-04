//! OpenAI-compatible LLM client with streaming SSE and retry logic.
//!
//! # How it works
//! `LlmClient` wraps `reqwest::Client` and speaks the OpenAI chat completions
//! protocol. It works with any backend that implements this API:
//! - Ollama (`localhost:11434/v1`)
//! - llama-server (`localhost:8080/v1`)
//! - mlx_lm.server (`localhost:8080/v1`)
//! - Any OpenAI-compatible endpoint
//!
//! # Sampling parameters
//! The client can inject model-specific sampling params (temperature, top_p, min_p,
//! repeat_penalty) into every request. These come from model profiles loaded by
//! the agent. Call `set_sampling()` after loading a profile.

use crate::message::{ChatRequest, ChatResponse};
use crate::retry::{self, RetryConfig};
use crate::stream::{StreamChunk, StreamEvent, ToolCallAccumulator};
use crate::usage::TokenUsage;
use anvil_config::{ProviderConfig, SamplingConfig};
use anyhow::{bail, Result};
use reqwest::Client;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// HTTP client for OpenAI-compatible chat completion APIs.
///
/// Handles authentication, retry with exponential backoff, SSE streaming,
/// and token usage tracking. Sampling parameters from model profiles are
/// injected into every request automatically.
pub struct LlmClient {
    http: Client,
    config: ProviderConfig,
    usage: TokenUsage,
    retry_config: RetryConfig,
    /// Model-specific sampling params, loaded from `.anvil/models/*.toml`.
    /// When set, these override the request's sampling fields.
    sampling: Option<SamplingConfig>,
}

impl LlmClient {
    /// Create a new client from provider configuration.
    ///
    /// # What happens
    /// 1. If `config.api_key` is set, adds `Authorization: Bearer <key>` header
    /// 2. Builds an HTTP client with rustls TLS
    /// 3. Initializes empty usage tracking and default retry config
    pub fn new(config: ProviderConfig) -> Result<Self> {
        let mut builder = Client::builder();
        if let Some(key) = config.resolve_api_key() {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", key).parse()?,
            );
            builder = builder.default_headers(headers);
        }
        let http = builder.build()?;
        Ok(Self {
            http,
            config,
            usage: TokenUsage::default(),
            retry_config: RetryConfig::default(),
            sampling: None,
        })
    }

    /// Get cumulative token usage across all requests in this session.
    pub fn usage(&self) -> &TokenUsage {
        &self.usage
    }

    /// Get the currently active model name.
    pub fn model(&self) -> &str {
        &self.config.model
    }

    /// Switch to a different model. Does not validate availability.
    pub fn set_model(&mut self, model: String) {
        self.config.model = model;
    }

    /// Get the base URL of the connected backend.
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Set the base URL (used when switching backends via `/backend` command).
    pub fn set_base_url(&mut self, url: String) {
        self.config.base_url = url;
    }

    /// Get the current backend kind.
    pub fn backend(&self) -> &anvil_config::BackendKind {
        &self.config.backend
    }

    /// Set the backend kind (used when switching backends).
    pub fn set_backend(&mut self, backend: anvil_config::BackendKind) {
        self.config.backend = backend;
    }

    /// Apply model-specific sampling parameters from a profile.
    ///
    /// # Why this exists
    /// Different models need different sampling params for optimal output.
    /// GLM-4.7-Flash needs temp=0.7, top_p=1.0, repeat_penalty=1.0.
    /// Devstral needs temp=0.15. These are injected into every request
    /// so the user doesn't have to remember per-model settings.
    pub fn set_sampling(&mut self, sampling: SamplingConfig) {
        self.sampling = Some(sampling);
    }

    /// Clear model-specific sampling (revert to backend defaults).
    pub fn clear_sampling(&mut self) {
        self.sampling = None;
    }

    /// Apply sampling config to a chat request.
    ///
    /// # Priority
    /// Profile sampling params are applied only when the request doesn't already
    /// have a value set. This lets per-request overrides take precedence.
    fn apply_sampling(&self, request: &mut ChatRequest) {
        if let Some(ref sampling) = self.sampling {
            if request.temperature.is_none() {
                request.temperature = sampling.temperature;
            }
            if request.top_p.is_none() {
                request.top_p = sampling.top_p;
            }
            if request.min_p.is_none() {
                request.min_p = sampling.min_p;
            }
            if request.repeat_penalty.is_none() {
                request.repeat_penalty = sampling.repeat_penalty;
            }
            if request.top_k.is_none() {
                request.top_k = sampling.top_k;
            }
        }
    }

    /// Send a non-streaming chat completion request.
    pub async fn chat(&mut self, request: &mut ChatRequest) -> Result<ChatResponse> {
        request.model.clone_from(&self.config.model);
        request.stream = false;
        self.apply_sampling(request);

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let resp = self.http.post(&url).json(request).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("LLM API error {status}: {body}");
        }

        let response: ChatResponse = resp.json().await?;
        if let Some(u) = &response.usage {
            self.usage.record(
                u.prompt_tokens,
                u.completion_tokens,
                self.config.pricing.as_ref(),
            );
        }
        Ok(response)
    }

    /// Send a streaming chat completion request with retry.
    ///
    /// # How retry works
    /// Only the initial HTTP POST is retried (not the SSE stream processing).
    /// Retryable conditions: 429 (rate limit), 500/502/503/504 (server errors),
    /// connection reset, timeout. Non-retryable: 400, 401, 403, 404.
    ///
    /// # tool_choice fallback
    /// Some backends (notably MLX) don't support `tool_choice`. If the initial
    /// request fails with 400/422 and the error mentions "tool_choice", the
    /// client retries once with `tool_choice` removed from the request body.
    ///
    /// # How streaming works
    /// Returns a channel receiver that emits `StreamEvent`s:
    /// - `ContentDelta` — text chunks as they arrive
    /// - `ToolCallDelta` — incremental tool call assembly
    /// - `Usage` — final token counts
    /// - `Done` — stream finished
    ///
    /// # Cancellation
    /// When the `cancel` token is triggered (e.g. by Ctrl+C), the SSE processing
    /// task stops reading and emits `StreamEvent::Done`. The HTTP response is
    /// dropped, which aborts the in-flight request.
    pub async fn chat_stream(
        &mut self,
        request: &mut ChatRequest,
        cancel: CancellationToken,
        mut on_retry: impl FnMut(usize, usize, std::time::Duration) + Send + 'static,
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        request.model.clone_from(&self.config.model);
        request.stream = true;
        self.apply_sampling(request);

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );

        let http = self.http.clone();
        let request_json = serde_json::to_value(&*request)?;
        let retry_config = self.retry_config.clone();

        let resp = retry::retry_async(&retry_config, &mut on_retry, || {
            let http = http.clone();
            let url = url.clone();
            let body = request_json.clone();
            async move {
                let resp = http.post(&url).json(&body).send().await.map_err(|e| {
                    let msg = e.to_string();
                    if retry::is_retryable_error(&msg) {
                        retry::RetryError::Retryable(anyhow::anyhow!("{e}"))
                    } else {
                        retry::RetryError::Permanent(anyhow::anyhow!("{e}"))
                    }
                })?;

                let status = resp.status().as_u16();
                if !resp.status().is_success() {
                    let body_text = resp.text().await.unwrap_or_default();
                    let err = anyhow::anyhow!("LLM API error {status}: {body_text}");
                    if retry::is_retryable_status(status) {
                        return Err(retry::RetryError::Retryable(err));
                    }
                    return Err(retry::RetryError::Permanent(err));
                }

                Ok(resp)
            }
        })
        .await;

        // tool_choice fallback: if the request failed with a 400/422 mentioning
        // "tool_choice", retry once without it. MLX and some other backends
        // don't support this parameter.
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                let err_str = e.to_string().to_lowercase();
                if (err_str.contains("400") || err_str.contains("422"))
                    && (err_str.contains("tool_choice") || err_str.contains("tool choice"))
                {
                    tracing::warn!(
                        "backend rejected tool_choice — retrying without it (MLX fallback)"
                    );
                    let mut fallback_body = request_json.clone();
                    if let Some(obj) = fallback_body.as_object_mut() {
                        obj.remove("tool_choice");
                    }
                    let resp = self.http.post(&url).json(&fallback_body).send().await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        bail!("LLM API error {status}: {body}");
                    }
                    resp
                } else {
                    return Err(e);
                }
            }
        };

        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            if let Err(e) = process_sse_stream(resp, &tx, cancel).await {
                tracing::error!("SSE stream error: {e}");
                let _ = tx.send(StreamEvent::Error(e.to_string())).await;
            }
            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }

    /// Update usage from a stream's final usage event.
    pub fn record_stream_usage(&mut self, prompt: u64, completion: u64) {
        self.usage
            .record(prompt, completion, self.config.pricing.as_ref());
    }
}

/// Process SSE stream, stopping early if the cancellation token fires.
///
/// When cancelled, the response is dropped (aborting the HTTP connection)
/// and any content received so far is preserved — the caller sees whatever
/// `ContentDelta` and `ToolCallDelta` events were already sent.
async fn process_sse_stream(
    resp: reqwest::Response,
    tx: &mpsc::Sender<StreamEvent>,
    cancel: CancellationToken,
) -> Result<()> {
    use tokio::io::AsyncBufReadExt;
    use tokio_stream::StreamExt;

    let byte_stream = resp.bytes_stream();
    let stream_reader =
        tokio_util::io::StreamReader::new(byte_stream.map(|r| r.map_err(std::io::Error::other)));
    let mut lines = tokio::io::BufReader::new(stream_reader).lines();

    let mut tool_acc = ToolCallAccumulator::default();

    loop {
        let line = tokio::select! {
            _ = cancel.cancelled() => {
                tracing::debug!("SSE stream cancelled");
                break;
            }
            result = lines.next_line() => {
                match result {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(e) => return Err(e.into()),
                }
            }
        };

        let line = line.trim().to_string();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let data = match line.strip_prefix("data: ") {
            Some(d) => d.trim(),
            None => continue,
        };
        if data == "[DONE]" {
            break;
        }

        let chunk: StreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to parse SSE chunk: {e}");
                continue;
            }
        };

        if let Some(usage) = &chunk.usage {
            let _ = tx.send(StreamEvent::Usage(usage.clone())).await;
        }

        for choice in &chunk.choices {
            if let Some(content) = &choice.delta.content {
                if !content.is_empty() {
                    let _ = tx.send(StreamEvent::ContentDelta(content.clone())).await;
                }
            }
            if let Some(tool_calls) = &choice.delta.tool_calls {
                for tc in tool_calls {
                    let args_delta = tc
                        .function
                        .as_ref()
                        .and_then(|f| f.arguments.as_deref())
                        .unwrap_or("");
                    let name = tc.function.as_ref().and_then(|f| f.name.clone());

                    tool_acc.push_delta(tc.index, tc.id.clone(), name.clone(), args_delta);

                    let _ = tx
                        .send(StreamEvent::ToolCallDelta {
                            index: tc.index,
                            id: tc.id.clone(),
                            name,
                            arguments_delta: args_delta.to_string(),
                        })
                        .await;
                }
            }
        }
    }

    Ok(())
}
