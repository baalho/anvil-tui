//! Configuration, harness directory management, and model profiles for Anvil.
//!
//! This crate owns everything related to Anvil's configuration:
//! - `.anvil/` harness directory (project-level config, skills, model profiles)
//! - `settings.toml` (provider, agent, and tool settings)
//! - Model profiles (per-model sampling parameters in `.anvil/models/`)
//! - Provider config (backend type, URL, API key)
//!
//! # Architecture
//! ```text
//! .anvil/
//! ├── config.toml          # Project settings (provider, agent, tools)
//! ├── context.md           # Injected into system prompt (lessons learned, project info)
//! ├── models/              # Per-model sampling profiles (TOML)
//! │   ├── glm-4.7-flash.toml
//! │   └── qwen3-coder.toml
//! ├── skills/              # Prompt template skills (Markdown with YAML frontmatter)
//! ├── inventory.toml       # Host/service registry (optional)
//! └── memory/              # Persistent learned patterns (categorized markdown)
//! ```

mod bundled_skills;
pub mod inventory;
pub mod migration;
mod profiles;
mod provider;
mod settings;

pub use bundled_skills::BUNDLED_SKILLS;
pub use inventory::{load_inventory, Inventory};
pub use profiles::{
    find_matching_profile, load_bundled_profiles, load_profiles, profiles_dir, BackendHints,
    Capabilities, ContextConfig, ModelProfile, SamplingConfig, BUNDLED_PROFILES,
};
pub use provider::{BackendKind, PricingConfig, ProviderConfig};
pub use settings::Settings;

use anyhow::Result;
use std::path::{Path, PathBuf};

const HARNESS_DIR: &str = ".anvil";
const CONFIG_FILE: &str = "config.toml";

/// Locate the `.anvil/` harness directory by walking upward from `start`.
///
/// # Why walk upward
/// Users may run `anvil` from a subdirectory of their project. Walking upward
/// finds the nearest `.anvil/` directory, similar to how `git` finds `.git/`.
/// Returns `None` if no harness directory exists in any ancestor.
pub fn find_harness_dir(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(HARNESS_DIR);
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Load settings from the harness directory, or return defaults.
///
/// # Resolution order
/// 1. Look for `.anvil/config.toml` in the working directory (or ancestors)
/// 2. If found, parse it as `Settings`
/// 3. If not found, return `Settings::default()` (Ollama on localhost:11434)
pub fn load_settings(working_dir: &Path) -> Result<Settings> {
    if let Some(harness) = find_harness_dir(working_dir) {
        let config_path = harness.join(CONFIG_FILE);
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let settings: Settings = toml::from_str(&content)?;
            return Ok(settings);
        }
    }
    Ok(Settings::default())
}

/// Scaffold a new `.anvil/` harness directory with default files.
///
/// # What gets created
/// - `config.toml` — default settings (Ollama, qwen3-coder:30b)
/// - `context.md` — project context template with lessons-learned section
/// - `models/` — bundled model profiles (Qwen3-Coder, Devstral, DeepSeek-R1, GLM-4.7)
/// - `skills/` — default verification and learning skills
/// - `memory/` — persistent learned patterns (categorized markdown)
///
/// Existing files are never overwritten — safe to run multiple times.
pub fn init_harness(dir: &Path) -> Result<PathBuf> {
    let harness = dir.join(HARNESS_DIR);
    std::fs::create_dir_all(&harness)?;

    // --- config.toml ---
    let config_path = harness.join(CONFIG_FILE);
    if !config_path.exists() {
        let defaults = Settings::default();
        let content = toml::to_string_pretty(&defaults)?;
        std::fs::write(&config_path, content)?;
    }

    // --- context.md with lessons-learned self-prompt ---
    let context_path = harness.join("context.md");
    if !context_path.exists() {
        std::fs::write(&context_path, DEFAULT_CONTEXT)?;
    }

    // --- subdirectories ---
    for subdir in ["skills", "memory"] {
        std::fs::create_dir_all(harness.join(subdir))?;
    }

    // --- model profiles ---
    let models_dir = harness.join("models");
    std::fs::create_dir_all(&models_dir)?;
    for (filename, content) in BUNDLED_PROFILES {
        let path = models_dir.join(filename);
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
    }

    // --- bundled skills ---
    let skills_dir = harness.join("skills");
    for (filename, content) in BUNDLED_SKILLS {
        let path = skills_dir.join(filename);
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
    }

    Ok(harness)
}

/// Return the user-level config directory (~/.config/anvil/).
pub fn user_config_dir() -> Result<PathBuf> {
    let config = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine user config directory"))?;
    let anvil_dir = config.join("anvil");
    std::fs::create_dir_all(&anvil_dir)?;
    Ok(anvil_dir)
}

/// Return the user-level data directory (~/.local/share/anvil/).
pub fn data_dir() -> Result<PathBuf> {
    let data =
        dirs::data_dir().ok_or_else(|| anyhow::anyhow!("cannot determine user data directory"))?;
    let anvil_dir = data.join("anvil");
    std::fs::create_dir_all(&anvil_dir)?;
    Ok(anvil_dir)
}

/// Default context.md content with lessons-learned self-prompt.
///
/// # Why this exists
/// This is the "prompt for yourself" — a self-reminder that Anvil reads on every
/// session start. It encodes hard-won lessons from development (the Ralph Loop
/// methodology, anti-patterns discovered during testing, model-specific quirks).
const DEFAULT_CONTEXT: &str = r#"# Project Context

Describe your project here. This content is injected into the system prompt.

## Lessons Learned (Self-Prompt)

These rules are injected into every session. Edit them as you discover new patterns.

- Shell commands must be strings, not argv arrays (models generate `ls -la`, not `["ls", "-la"]`)
- Always read a file before editing — understand structure before making changes
- Prefer `file_edit` (search/replace) over `file_write` for existing files
- Check exit codes before proceeding — a silent failure cascades into worse failures
- When stuck in a loop, try a different approach — don't repeat the same failing command
- Verify changes with the appropriate tool before declaring done
- For GLM-4.7-Flash: use temp=0.7, top_p=1.0, disable repeat penalty (1.0)
- For autonomous mode: read verification output carefully before next iteration
- Keep tool calls focused — one action per call, examine results before proceeding
"#;
