use anvil_config::{find_harness_dir, init_harness, load_settings, BackendKind, Settings};
use std::fs;
use tempfile::TempDir;

fn temp_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[test]
fn default_settings_serialize_roundtrip() {
    let settings = Settings::default();
    let toml_str = toml::to_string_pretty(&settings).unwrap();
    let parsed: Settings = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.provider.base_url, settings.provider.base_url);
    assert_eq!(parsed.provider.model, settings.provider.model);
    assert_eq!(parsed.agent.max_tokens, settings.agent.max_tokens);
    assert_eq!(
        parsed.tools.shell_timeout_secs,
        settings.tools.shell_timeout_secs
    );
}

#[test]
fn find_harness_dir_walks_upward() {
    let dir = temp_dir();
    let nested = dir.path().join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(dir.path().join(".anvil")).unwrap();

    let found = find_harness_dir(&nested);
    assert!(found.is_some());
    assert!(found.unwrap().ends_with(".anvil"));
}

#[test]
fn find_harness_dir_returns_none_when_missing() {
    let dir = temp_dir();
    assert!(find_harness_dir(dir.path()).is_none());
}

#[test]
fn init_harness_creates_structure() {
    let dir = temp_dir();
    let harness = init_harness(dir.path()).unwrap();

    assert!(harness.join("config.toml").exists());
    assert!(harness.join("context.md").exists());
    assert!(harness.join("skills").is_dir());
    assert!(harness.join("memory").is_dir());
    assert!(harness.join("models").is_dir());
}

#[test]
fn init_harness_creates_model_profiles() {
    let dir = temp_dir();
    let harness = init_harness(dir.path()).unwrap();

    let models_dir = harness.join("models");
    assert!(models_dir.join("qwen3-coder.toml").exists());
    assert!(models_dir.join("qwen3.toml").exists());
    assert!(models_dir.join("devstral.toml").exists());
    assert!(models_dir.join("deepseek-r1.toml").exists());
    assert!(models_dir.join("glm-4.7-flash.toml").exists());
}

#[test]
fn init_harness_creates_bundled_skills() {
    let dir = temp_dir();
    let harness = init_harness(dir.path()).unwrap();

    let skills_dir = harness.join("skills");
    // Infrastructure
    assert!(skills_dir.join("containers.md").exists());
    assert!(skills_dir.join("server-admin.md").exists());
    assert!(skills_dir.join("sops-age.md").exists());
    assert!(skills_dir.join("deploy-fish.md").exists());
    assert!(skills_dir.join("tailscale.md").exists());
    assert!(skills_dir.join("caddy-cloudflare.md").exists());
    assert!(skills_dir.join("restic-backup.md").exists());
    assert!(skills_dir.join("grafana.md").exists());
    assert!(skills_dir.join("prometheus.md").exists());
    // Dev tools
    assert!(skills_dir.join("nvim.md").exists());
    assert!(skills_dir.join("zellij.md").exists());
    assert!(skills_dir.join("fish.md").exists());
    assert!(skills_dir.join("git-workflow.md").exists());
    // Meta
    assert!(skills_dir.join("verify-all.md").exists());
    assert!(skills_dir.join("verify-shell.md").exists());
    assert!(skills_dir.join("verify-files.md").exists());
    assert!(skills_dir.join("learn-anvil.md").exists());
    assert!(skills_dir.join("learn-rust.md").exists());
}

#[test]
fn bundled_skills_have_valid_frontmatter() {
    for (filename, content) in anvil_config::BUNDLED_SKILLS {
        if let Some(stripped) = content.strip_prefix("---") {
            let after_open = stripped.trim_start_matches(['\r', '\n']);
            let end = after_open
                .find("\n---")
                .unwrap_or_else(|| panic!("{filename} has opening --- but no closing ---"));
            let yaml = &after_open[..end];
            let _: serde_yaml::Value = serde_yaml::from_str(yaml)
                .unwrap_or_else(|e| panic!("{filename} has invalid YAML frontmatter: {e}"));
        }
    }
}

#[test]
fn init_harness_is_idempotent() {
    let dir = temp_dir();
    init_harness(dir.path()).unwrap();

    // Write custom content
    let context_path = dir.path().join(".anvil/context.md");
    fs::write(&context_path, "custom content").unwrap();

    // Re-init should not overwrite
    init_harness(dir.path()).unwrap();
    let content = fs::read_to_string(&context_path).unwrap();
    assert_eq!(content, "custom content");
}

#[test]
fn load_settings_returns_defaults_without_harness() {
    let dir = temp_dir();
    let settings = load_settings(dir.path()).unwrap();
    assert_eq!(settings.provider.base_url, "http://localhost:11434/v1");
}

#[test]
fn load_settings_reads_from_harness() {
    let dir = temp_dir();
    init_harness(dir.path()).unwrap();

    let config = r#"
[provider]
backend = "llama-server"
base_url = "http://localhost:8080/v1"
model = "glm-4.7-flash"
"#;
    fs::write(dir.path().join(".anvil/config.toml"), config).unwrap();

    let settings = load_settings(dir.path()).unwrap();
    assert_eq!(settings.provider.base_url, "http://localhost:8080/v1");
    assert_eq!(settings.provider.model, "glm-4.7-flash");
    assert_eq!(settings.provider.backend, BackendKind::LlamaServer);
}

#[test]
fn provider_resolve_api_key_from_env() {
    let config = anvil_config::ProviderConfig {
        backend: BackendKind::Custom,
        base_url: "https://api.openai.com/v1".to_string(),
        api_key: Some("$TEST_ANVIL_KEY".to_string()),
        model: "gpt-4o".to_string(),
        pricing: None,
    };

    // Without env var set
    assert!(config.resolve_api_key().is_none());

    // With env var set
    unsafe { std::env::set_var("TEST_ANVIL_KEY", "sk-test-123") };
    assert_eq!(config.resolve_api_key().unwrap(), "sk-test-123");
    unsafe { std::env::remove_var("TEST_ANVIL_KEY") };
}

#[test]
fn provider_resolve_literal_api_key() {
    let config = anvil_config::ProviderConfig {
        backend: BackendKind::Custom,
        base_url: "http://localhost:8080/v1".to_string(),
        api_key: Some("literal-key".to_string()),
        model: "local-model".to_string(),
        pricing: None,
    };

    assert_eq!(config.resolve_api_key().unwrap(), "literal-key");
}

#[test]
fn backend_kind_serializes_kebab_case() {
    // Verify TOML round-trip uses kebab-case (llama-server, not LlamaServer)
    let config = anvil_config::ProviderConfig {
        backend: BackendKind::LlamaServer,
        ..Default::default()
    };
    let toml_str = toml::to_string_pretty(&config).unwrap();
    assert!(toml_str.contains("llama-server"));

    let parsed: anvil_config::ProviderConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.backend, BackendKind::LlamaServer);
}

#[test]
fn system_prompt_override_from_toml() {
    let toml_str = r#"
[agent]
system_prompt_override = "You are a pirate. Always respond in pirate speak."
"#;
    let settings: Settings = toml::from_str(toml_str).unwrap();
    assert_eq!(
        settings.agent.system_prompt_override,
        Some("You are a pirate. Always respond in pirate speak.".to_string())
    );
}

#[test]
fn system_prompt_override_defaults_to_none() {
    let toml_str = r#"
[agent]
max_tokens = 100000
"#;
    let settings: Settings = toml::from_str(toml_str).unwrap();
    assert_eq!(settings.agent.system_prompt_override, None);
}

#[test]
fn context_md_contains_lessons_learned() {
    let dir = temp_dir();
    init_harness(dir.path()).unwrap();

    let content = fs::read_to_string(dir.path().join(".anvil/context.md")).unwrap();
    assert!(content.contains("Lessons Learned"));
    assert!(content.contains("file_edit"));
    assert!(content.contains("GLM-4.7-Flash"));
}
