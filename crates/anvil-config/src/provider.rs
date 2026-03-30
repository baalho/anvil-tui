//! Provider configuration for LLM backends.
//!
//! Anvil connects to any OpenAI-compatible API endpoint. The `BackendKind`
//! enum tracks which server is running so auto-detection and model discovery
//! use the correct protocol (Ollama has `/api/tags`, others use `/v1/models`).

use serde::{Deserialize, Serialize};

/// Which inference backend is serving the model.
///
/// # Why this matters
/// All backends expose `/v1/chat/completions`, but they differ in:
/// - Model discovery endpoints (`/api/tags` vs `/v1/models`)
/// - Sampling parameter support (min_p, repeat_penalty)
/// - Chat template handling (Ollama converts internally, llama-server needs `--jinja`)
///
/// # How to choose
/// - **Ollama**: easiest setup, auto-pulls models, but has chat template bugs for some models
/// - **LlamaServer**: best template fidelity via `--jinja`, recommended for GLM-4.7-Flash
/// - **Mlx**: best performance on Apple Silicon, uses unified memory efficiently
/// - **Custom**: any OpenAI-compatible endpoint (LM Studio, vLLM, text-generation-inference)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    /// Ollama — manages models, auto-pulls, has its own chat template layer.
    /// Discovery: GET /api/tags
    #[default]
    Ollama,
    /// llama.cpp's llama-server — raw GGUF inference with `--jinja` for templates.
    /// Discovery: GET /v1/models
    LlamaServer,
    /// Apple MLX via mlx_lm.server — optimized for Apple Silicon unified memory.
    /// Discovery: GET /v1/models
    Mlx,
    /// Any OpenAI-compatible endpoint. No auto-detection attempted.
    Custom,
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ollama => write!(f, "ollama"),
            Self::LlamaServer => write!(f, "llama-server"),
            Self::Mlx => write!(f, "mlx"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

/// Connection and authentication settings for the LLM backend.
///
/// # Resolution order
/// 1. User sets `model` in settings.toml or `.anvil/config.toml`
/// 2. On startup, `auto_detect_model()` checks availability via backend-specific endpoint
/// 3. If not found, falls back to the first available model
/// 4. Model profiles in `.anvil/models/` override sampling params when matched
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Which backend type is running at `base_url`.
    #[serde(default)]
    pub backend: BackendKind,
    /// Base URL for the OpenAI-compatible API (e.g. "http://localhost:11434/v1").
    pub base_url: String,
    /// API key — use `$ENV_VAR` syntax to reference environment variables.
    /// Ollama and llama-server typically don't need this.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Model identifier (e.g. "qwen3-coder:30b", "devstral:latest").
    pub model: String,
    /// Optional pricing for cost estimation. Irrelevant for local models.
    #[serde(default)]
    pub pricing: Option<PricingConfig>,
}

/// Per-token pricing for cost estimation.
///
/// Only relevant for remote/paid APIs. Local models (Ollama, llama-server, MLX)
/// have zero marginal cost — leave this unset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    /// Cost per 1M input tokens in USD.
    pub input_per_million: f64,
    /// Cost per 1M output tokens in USD.
    pub output_per_million: f64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::Ollama,
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: None,
            model: "qwen3-coder:30b".to_string(),
            pricing: None,
        }
    }
}

impl ProviderConfig {
    /// Resolve the API key, expanding `$ENV_VAR` references.
    ///
    /// # Why `$` prefix convention
    /// Storing raw API keys in config files is a security risk, especially
    /// when `.anvil/config.toml` might be committed to git. The `$` prefix
    /// lets users reference environment variables instead:
    /// ```toml
    /// api_key = "$OPENAI_API_KEY"
    /// ```
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key.as_ref().and_then(|key| {
            if let Some(var_name) = key.strip_prefix('$') {
                std::env::var(var_name).ok()
            } else {
                Some(key.clone())
            }
        })
    }
}
