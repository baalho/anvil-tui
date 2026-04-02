//! Model profiles — per-model sampling parameters and backend hints.
//!
//! # Why model profiles exist
//! Different LLMs need different sampling parameters for optimal output.
//! GLM-4.7-Flash needs `temp=0.7, top_p=1.0, repeat_penalty=1.0` for tool calling.
//! Devstral works best with `temp=0.15`. Qwen models use defaults.
//!
//! Without profiles, users must remember and manually configure these per model.
//! Profiles automate this: when Anvil detects a model name, it loads the matching
//! profile and injects the correct sampling params into every API request.
//!
//! # How matching works
//! Each profile has `match_patterns` — a list of substrings. When the active model
//! name contains any pattern (case-insensitive), that profile activates.
//! Example: patterns `["glm-4.7-flash", "GLM-4.7-Flash"]` match both
//! `glm-4.7-flash:latest` (Ollama) and `GLM-4.7-Flash-Q4_K_M` (llama-server).
//!
//! # File format
//! Profiles live in `.anvil/models/*.toml`:
//! ```toml
//! name = "GLM-4.7-Flash"
//! match_patterns = ["glm-4.7-flash", "GLM-4.7-Flash"]
//!
//! [sampling]
//! temperature = 0.7
//! top_p = 1.0
//! min_p = 0.01
//! repeat_penalty = 1.0
//!
//! [context]
//! max_window = 202752
//! default_window = 16384
//!
//! [backend]
//! preferred = "llama-server"
//! flags = ["--jinja"]
//! notes = "Unsloth warns against Ollama for this model"
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A model profile with sampling parameters and backend hints.
///
/// Loaded from `.anvil/models/<name>.toml`. Matched against the active model
/// name to automatically apply optimal settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Human-readable model name (e.g. "GLM-4.7-Flash").
    pub name: String,
    /// Substrings to match against the active model identifier.
    /// Case-insensitive matching. First match wins.
    pub match_patterns: Vec<String>,
    /// Sampling parameters for inference.
    #[serde(default)]
    pub sampling: SamplingConfig,
    /// Context window settings.
    #[serde(default)]
    pub context: ContextConfig,
    /// Backend-specific hints (which server works best, launch flags).
    #[serde(default)]
    pub backend: BackendHints,
    /// What this model is good at. Metadata for `/model` display and
    /// future auto-selection. Missing section is fine (backward compatible).
    #[serde(default)]
    pub capabilities: Capabilities,
}

/// What a model is good at — metadata for display and future auto-routing.
///
/// Strengths are free-form strings like "coding", "creative", "reasoning",
/// "tool-calling". Displayed by `/model` so users can pick the right model
/// for their task. Future versions may use these for automatic mode→model mapping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// What this model excels at (e.g. ["coding", "tool-calling"]).
    #[serde(default)]
    pub strengths: Vec<String>,
}

/// Sampling parameters injected into chat completion requests.
///
/// # Why these specific fields
/// These are the parameters that vary most across models and have the biggest
/// impact on output quality. Values come from model authors' recommendations
/// (e.g. Z.ai recommends temp=0.7 for GLM-4.7 tool calling).
///
/// All fields are optional — `None` means "use the backend's default".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamplingConfig {
    /// Controls randomness. Lower = more deterministic.
    /// GLM-4.7 tool calling: 0.7. Devstral: 0.15. Most models: 0.7-1.0.
    pub temperature: Option<f32>,
    /// Nucleus sampling threshold. Considers tokens whose cumulative probability
    /// exceeds this value. GLM-4.7 tool calling: 1.0 (disabled).
    pub top_p: Option<f32>,
    /// Minimum probability threshold. Filters tokens below this probability.
    /// llama.cpp default is 0.05, but GLM-4.7 needs 0.01.
    pub min_p: Option<f32>,
    /// Penalizes repeated tokens. Set to 1.0 to disable.
    /// GLM-4.7 specifically requires this disabled (1.0) to avoid looping.
    pub repeat_penalty: Option<f32>,
    /// Limits sampling to top-K most probable tokens.
    pub top_k: Option<u32>,
}

/// Context window configuration for the model.
///
/// # Why two values
/// `max_window` is the model's architectural limit (e.g. 202,752 for GLM-4.7-Flash).
/// `default_window` is a practical default for most tasks — using the full window
/// wastes memory and slows inference when you don't need 200K context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Maximum context window the model supports (architectural limit).
    pub max_window: usize,
    /// Practical default for everyday use. Anvil uses this unless overridden.
    pub default_window: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_window: 8192,
            default_window: 8192,
        }
    }
}

/// Hints about which backend works best for this model.
///
/// These are informational — Anvil doesn't enforce them. They help users
/// make informed choices and document known compatibility issues.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendHints {
    /// Recommended backend (e.g. "llama-server", "ollama", "mlx").
    pub preferred: Option<String>,
    /// Extra flags needed when launching the backend (e.g. ["--jinja"]).
    #[serde(default)]
    pub flags: Vec<String>,
    /// Human-readable notes about compatibility or quirks.
    pub notes: Option<String>,
}

/// Loads all model profiles from a directory.
///
/// # How it works
/// Scans `profiles_dir` for `*.toml` files, parses each as a `ModelProfile`,
/// and returns them sorted by name. Invalid files are logged and skipped —
/// one bad profile shouldn't prevent Anvil from starting.
pub fn load_profiles(profiles_dir: &Path) -> Vec<ModelProfile> {
    let dir = match std::fs::read_dir(profiles_dir) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut profiles = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match load_single_profile(&path) {
            Ok(profile) => profiles.push(profile),
            Err(e) => {
                tracing::warn!("skipping invalid model profile {}: {e}", path.display());
            }
        }
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    profiles
}

/// Parse a single TOML profile file.
fn load_single_profile(path: &Path) -> Result<ModelProfile> {
    let content = std::fs::read_to_string(path)?;
    let profile: ModelProfile = toml::from_str(&content)?;
    Ok(profile)
}

/// Find the profile that matches a given model name.
///
/// # Matching algorithm
/// Case-insensitive substring match against each profile's `match_patterns`.
/// First profile with any matching pattern wins. This handles the common case
/// where Ollama appends `:latest` and llama-server uses the full GGUF filename.
///
/// Example: model "glm-4.7-flash:latest" matches pattern "glm-4.7-flash".
pub fn find_matching_profile<'a>(
    profiles: &'a [ModelProfile],
    model_name: &str,
) -> Option<&'a ModelProfile> {
    let model_lower = model_name.to_lowercase();
    profiles.iter().find(|p| {
        p.match_patterns
            .iter()
            .any(|pattern| model_lower.contains(&pattern.to_lowercase()))
    })
}

/// Returns the directory path for model profiles within a harness.
/// Load all bundled model profiles (compiled into the binary).
///
/// Useful when no `.anvil/models/` directory exists or when the caller
/// needs profiles without filesystem access (e.g., during `/model` command).
pub fn load_bundled_profiles() -> Vec<ModelProfile> {
    BUNDLED_PROFILES
        .iter()
        .filter_map(|(_, content)| toml::from_str(content).ok())
        .collect()
}

pub fn profiles_dir(harness_dir: &Path) -> PathBuf {
    harness_dir.join("models")
}

// ---------------------------------------------------------------------------
// Bundled profile content — shipped with `anvil init`
// ---------------------------------------------------------------------------

/// Bundled model profiles created by `anvil init`.
///
/// # Why bundle these
/// Users shouldn't need to research optimal sampling params for popular models.
/// These defaults come from model authors' official recommendations and
/// community-tested settings. Updated to reflect the current model landscape.
///
/// # Models included (sorted by relevance for coding agents)
/// - Qwen3-Coder 30B: best open-source coding agent model (MoE, 3.3B active)
/// - Qwen3 (general): latest Qwen generation, replaces Qwen2.5 entirely
/// - Devstral 24B: #1 on SWE-Bench Verified among open-source models
/// - DeepSeek-R1: reasoning model, strong at code with chain-of-thought
/// - GLM-4.7-Flash: Z.ai's MoE model, good tool calling
///
/// # 64GB M4 Max capacity
/// All bundled profiles fit comfortably in 64GB unified memory at Q4/Q5.
/// Qwen3-Coder 30B (19GB), Devstral 24B (14GB), DeepSeek-R1 32B (20GB).
pub const BUNDLED_PROFILES: &[(&str, &str)] = &[
    (
        "qwen3-coder.toml",
        r#"# Qwen3-Coder 30B — Alibaba's coding agent model (July 2025)
# Source: https://qwenlm.github.io/blog/qwen3-coder/
# 30B total params, 3.3B active (MoE). 256K native context.
# Trained with execution-driven RL on SWE-Bench. Best open-source coding agent.
# Fits on 64GB M4 Max (19GB at Q4_K_M).

name = "Qwen3-Coder"
match_patterns = ["qwen3-coder", "Qwen3-Coder"]

[sampling]
temperature = 0.7
top_p = 0.95

[context]
max_window = 262144
default_window = 32768

[backend]
preferred = "ollama"
notes = "Works well with Ollama. For 256K context, ensure sufficient memory."

[capabilities]
strengths = ["coding", "tool-calling"]
"#,
    ),
    (
        "qwen3.toml",
        r#"# Qwen3 — Alibaba's latest general-purpose model family (May 2025)
# Source: https://qwenlm.github.io/blog/qwen3/
# Replaces Qwen2.5 entirely. Available 0.6B to 235B.
# Supports thinking mode (chain-of-thought) and tool calling.
# 30B MoE (19GB) and 32B dense (20GB) both fit 64GB M4 Max.

name = "Qwen3"
match_patterns = ["qwen3:", "qwen3-"]

[sampling]
temperature = 0.7
top_p = 0.9

[context]
max_window = 262144
default_window = 16384

[backend]
preferred = "ollama"

[capabilities]
strengths = ["creative", "reasoning", "tool-calling"]
"#,
    ),
    (
        "qwen2.5-coder.toml",
        r#"# Qwen2.5-Coder — Alibaba's coding model (Nov 2024)
# Source: https://qwenlm.github.io/blog/qwen2.5-coder-family/
# Available 0.5B, 1.5B, 3B, 7B, 14B, 32B. 128K native context.
# Good tool calling support. 32B (20GB) fits 64GB M4 Max.
# Lower temperature than Qwen3 for more deterministic file generation.

name = "Qwen2.5-Coder"
match_patterns = ["qwen2.5-coder", "Qwen2.5-Coder"]

[sampling]
temperature = 0.3
top_p = 0.9

[context]
max_window = 131072
default_window = 16384

[backend]
preferred = "ollama"
notes = "Use temperature 0.3 for reliable tool calling and file generation."

[capabilities]
strengths = ["coding", "tool-calling"]
"#,
    ),
    (
        "qwen2.5.toml",
        r#"# Qwen2.5 — Alibaba's general-purpose model family (Sep 2024)
# Source: https://qwenlm.github.io/blog/qwen2.5/
# Available 0.5B to 72B. 128K native context.
# Supports tool calling. 32B (20GB) fits 64GB M4 Max.

name = "Qwen2.5"
match_patterns = ["qwen2.5:", "qwen2.5-"]

[sampling]
temperature = 0.5
top_p = 0.9

[context]
max_window = 131072
default_window = 16384

[backend]
preferred = "ollama"

[capabilities]
strengths = ["creative", "reasoning", "tool-calling"]
"#,
    ),
    (
        "devstral.toml",
        r#"# Devstral 24B — Mistral x All Hands AI coding agent model
# Source: https://mistral.ai/news/devstral
# 24B params, 128K context. #1 open-source on SWE-Bench Verified (46.8%).
# Fine-tuned from Mistral Small 3.1. Text-only (vision encoder removed).
# Fits easily on 64GB M4 Max (14GB at Q4_K_M).

name = "Devstral"
match_patterns = ["devstral"]

[sampling]
temperature = 0.15

[context]
max_window = 131072
default_window = 16384

[backend]
preferred = "ollama"
notes = "Apache 2.0 license. Use llama-server with --jinja for best results."

[capabilities]
strengths = ["coding", "tool-calling"]
"#,
    ),
    (
        "deepseek-r1.toml",
        r#"# DeepSeek-R1 — reasoning model with chain-of-thought
# Source: https://github.com/deepseek-ai/DeepSeek-R1
# Updated to R1-0528. Performance approaching O3 and Gemini 2.5 Pro.
# Distilled versions: 1.5B, 7B, 8B, 14B, 32B, 70B. Full: 671B.
# 32B distill (20GB) fits 64GB M4 Max. 14B (9GB) for faster iteration.

name = "DeepSeek-R1"
match_patterns = ["deepseek-r1", "deepseek-r1-distill"]

[sampling]
temperature = 0.6
top_p = 0.95

[context]
max_window = 131072
default_window = 16384

[backend]
preferred = "ollama"
notes = "Reasoning model — outputs <think> blocks before answering. MIT license."

[capabilities]
strengths = ["reasoning", "coding"]
"#,
    ),
    (
        "qwen3.5.toml",
        r#"# Qwen3.5 — Alibaba's hybrid reasoning model family (Jul 2025)
# Source: https://unsloth.ai/docs/models/qwen3.5
# Sizes: 0.8B, 2B, 4B, 9B, 27B, 35B-A3B (MoE), 122B-A10B, 397B-A17B.
# 256K native context, 201 languages, thinking + non-thinking modes.
# 35B-A3B (22GB Q4) and 27B (17GB Q4) fit 64GB M4 Max.
# NOTE: Currently no Ollama GGUF support — use llama-server with --jinja.

name = "Qwen3.5"
match_patterns = ["qwen3.5"]

[sampling]
temperature = 0.6
top_p = 0.95
top_k = 20
repeat_penalty = 1.0

[context]
max_window = 262144
default_window = 32768

[backend]
preferred = "llama-server"
flags = ["--jinja"]
notes = "No Ollama GGUF support yet (mmproj vision files). Use llama-server with --jinja."

[capabilities]
strengths = ["creative", "reasoning", "coding", "tool-calling"]
"#,
    ),
    (
        "nemotron-cascade-2.toml",
        r#"# Nemotron Cascade 2 — NVIDIA's 30B MoE reasoning + agentic model (Jul 2025)
# Source: https://ollama.com/library/nemotron-cascade-2
# 30B total, 3B active (MoE). 256K context. 24GB on disk.
# Gold medal IMO + IOI performance. Thinking + instruct modes.
# Supports tool calling. Fits on 64GB M4 Max.

name = "Nemotron-Cascade-2"
match_patterns = ["nemotron-cascade", "nemotron_cascade"]

[sampling]
temperature = 0.6
top_p = 0.95

[context]
max_window = 262144
default_window = 32768

[backend]
preferred = "ollama"
notes = "Supports thinking mode and tool calling. Strong reasoning and agentic capabilities."

[capabilities]
strengths = ["reasoning", "coding", "tool-calling"]
"#,
    ),
    (
        "glm-4.7-flash.toml",
        r#"# GLM-4.7-Flash — Z.ai's 30B MoE reasoning model
# Source: https://unsloth.ai/docs/models/glm-4.7-flash
# ~3.6B active parameters, 200K context.
# Fits on 64GB M4 Max (18GB at Q4_K_M).

name = "GLM-4.7-Flash"
match_patterns = ["glm-4.7-flash", "GLM-4.7-Flash", "glm-4.7"]

[sampling]
temperature = 0.7
top_p = 1.0
min_p = 0.01
repeat_penalty = 1.0  # MUST be 1.0 — disabling repeat penalty is required

[context]
max_window = 202752
default_window = 16384

[backend]
preferred = "llama-server"
flags = ["--jinja"]
notes = "Unsloth warns against Ollama due to chat template bugs. Use llama-server with --jinja."

[capabilities]
strengths = ["creative", "tool-calling"]
"#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_bundled_profiles() {
        // Every bundled profile must parse without error
        for (filename, content) in BUNDLED_PROFILES {
            let profile: ModelProfile = toml::from_str(content)
                .unwrap_or_else(|e| panic!("failed to parse {filename}: {e}"));
            assert!(!profile.name.is_empty(), "{filename} has empty name");
            assert!(
                !profile.match_patterns.is_empty(),
                "{filename} has no match patterns"
            );
        }
    }

    #[test]
    fn load_profiles_from_directory() {
        let dir = TempDir::new().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        for (filename, content) in BUNDLED_PROFILES {
            std::fs::write(models_dir.join(filename), content).unwrap();
        }

        let profiles = load_profiles(&models_dir);
        assert_eq!(profiles.len(), BUNDLED_PROFILES.len());
    }

    #[test]
    fn match_profile_case_insensitive() {
        let profiles = BUNDLED_PROFILES
            .iter()
            .map(|(_, content)| toml::from_str::<ModelProfile>(content).unwrap())
            .collect::<Vec<_>>();

        // Ollama-style name with :latest suffix
        let matched = find_matching_profile(&profiles, "glm-4.7-flash:latest");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "GLM-4.7-Flash");

        // llama-server style with GGUF filename
        let matched = find_matching_profile(&profiles, "GLM-4.7-Flash-Q4_K_M.gguf");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "GLM-4.7-Flash");

        // Qwen3-Coder match
        let matched = find_matching_profile(&profiles, "qwen3-coder:30b");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Qwen3-Coder");

        // DeepSeek-R1 match
        let matched = find_matching_profile(&profiles, "deepseek-r1:32b");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "DeepSeek-R1");

        // Devstral match
        let matched = find_matching_profile(&profiles, "devstral:latest");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Devstral");
    }

    #[test]
    fn no_match_returns_none() {
        let profiles = BUNDLED_PROFILES
            .iter()
            .map(|(_, content)| toml::from_str::<ModelProfile>(content).unwrap())
            .collect::<Vec<_>>();

        // Qwen2.5 family
        let matched = find_matching_profile(&profiles, "qwen2.5-coder:32b");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Qwen2.5-Coder");

        let matched = find_matching_profile(&profiles, "qwen2.5:7b");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Qwen2.5");

        // Qwen3.5
        let matched = find_matching_profile(&profiles, "qwen3.5-35b-a3b:latest");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Qwen3.5");

        // Nemotron Cascade 2
        let matched = find_matching_profile(&profiles, "nemotron-cascade-2:latest");
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().name, "Nemotron-Cascade-2");

        assert!(find_matching_profile(&profiles, "unknown-model-xyz").is_none());
    }

    #[test]
    fn sampling_defaults_are_none() {
        let config = SamplingConfig::default();
        assert!(config.temperature.is_none());
        assert!(config.top_p.is_none());
        assert!(config.min_p.is_none());
        assert!(config.repeat_penalty.is_none());
        assert!(config.top_k.is_none());
    }

    #[test]
    fn skip_invalid_profile_files() {
        let dir = TempDir::new().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();

        // Valid profile
        std::fs::write(models_dir.join("good.toml"), BUNDLED_PROFILES[0].1).unwrap();

        // Invalid TOML
        std::fs::write(models_dir.join("bad.toml"), "this is not valid toml {{{}").unwrap();

        // Non-TOML file (should be ignored)
        std::fs::write(models_dir.join("readme.md"), "# Models").unwrap();

        let profiles = load_profiles(&models_dir);
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn empty_directory_returns_empty() {
        let dir = TempDir::new().unwrap();
        let profiles = load_profiles(dir.path());
        assert!(profiles.is_empty());
    }

    #[test]
    fn nonexistent_directory_returns_empty() {
        let profiles = load_profiles(Path::new("/nonexistent/path/models"));
        assert!(profiles.is_empty());
    }
}
