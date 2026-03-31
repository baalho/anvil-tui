//! Skill loading and parsing — markdown prompt templates with optional YAML frontmatter.
//!
//! # What is a skill?
//! A skill is a markdown file in `.anvil/skills/` that provides domain-specific
//! instructions to the LLM. When activated via `/skill <name>`, the skill's content
//! is injected into the system prompt, giving the LLM specialized knowledge.
//!
//! # Frontmatter format
//! Skills can optionally include YAML frontmatter between `---` delimiters:
//! ```markdown
//! ---
//! description: "Manage Docker containers and compose stacks"
//! category: infrastructure
//! tags: [docker, containers]
//! env:
//!   - DOCKER_HOST
//!   - DOCKER_CONFIG
//! verify: "docker info --format '{{.ServerVersion}}'"
//! ---
//! # Docker Management
//!
//! ...prompt content...
//! ```
//!
//! # Backward compatibility
//! Skills without frontmatter work exactly as before — the heading becomes the name,
//! the first non-empty line becomes the description, and everything after the heading
//! is the prompt content.

use anyhow::{bail, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A parsed skill with metadata and prompt content.
///
/// The `content` field is what gets injected into the system prompt.
/// Metadata fields (`category`, `tags`, `required_env`, `verify_command`)
/// are used by the UI and agent for organization and env passthrough.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Filename without extension (e.g. "docker" from "docker.md").
    pub key: String,
    /// Human-readable name from the `# Heading` or frontmatter.
    pub name: String,
    /// One-line description for `/skill` listing.
    pub description: String,
    /// The prompt content injected into the system prompt.
    pub content: String,
    /// Organizational category (e.g. "infrastructure", "dev-tools", "meta").
    pub category: Option<String>,
    /// Searchable tags for future filtering.
    pub tags: Vec<String>,
    /// Environment variables this skill needs passed to the shell tool.
    /// When the skill is active, these vars are added to the shell's env allowlist.
    pub required_env: Vec<String>,
    /// Shell command to verify the skill's prerequisites are met.
    /// Run via `/skill verify <name>`. Exit 0 = pass, non-zero = fail.
    pub verify_command: Option<String>,
    /// Other skill keys this skill depends on.
    /// Activating this skill auto-activates all dependencies.
    pub depends: Vec<String>,
}

/// YAML frontmatter structure, deserialized from the `---` block.
///
/// All fields are optional — a skill can have partial frontmatter.
/// Missing fields fall back to heading-based extraction.
#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    description: Option<String>,
    category: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    verify: Option<String>,
    /// Other skills this one depends on. Activating this skill
    /// auto-activates all dependencies (transitively).
    #[serde(default)]
    depends: Vec<String>,
}

/// Scans `.anvil/skills/` and loads all valid skill files.
pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    /// Create a loader for the given workspace's skills directory.
    pub fn new(workspace: &Path) -> Self {
        Self {
            skills_dir: workspace.join(".anvil").join("skills"),
        }
    }

    /// Scan the skills directory and return all parseable skills, sorted by key.
    ///
    /// Invalid files are silently skipped — one bad skill shouldn't break the listing.
    pub fn scan(&self) -> Vec<Skill> {
        let mut skills = Vec::new();
        let dir = match std::fs::read_dir(&self.skills_dir) {
            Ok(d) => d,
            Err(_) => return skills,
        };

        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            if let Ok(skill) = parse_skill_file(&path) {
                skills.push(skill);
            }
        }

        skills.sort_by(|a, b| a.key.cmp(&b.key));
        skills
    }

    /// Load a specific skill by key (filename without extension).
    pub fn get(&self, key: &str) -> Result<Skill> {
        let path = self.skills_dir.join(format!("{key}.md"));
        if !path.exists() {
            anyhow::bail!("skill not found: {key}");
        }
        parse_skill_file(&path)
    }

    /// Resolve transitive dependencies for a skill.
    /// Returns the list of skill keys to activate (including the skill itself),
    /// in dependency-first order. Detects circular dependencies.
    pub fn resolve_dependencies(&self, key: &str) -> Result<Vec<String>> {
        let mut resolved = Vec::new();
        let mut visiting = HashSet::new();
        self.resolve_recursive(key, &mut resolved, &mut visiting)?;
        Ok(resolved)
    }

    fn resolve_recursive(
        &self,
        key: &str,
        resolved: &mut Vec<String>,
        visiting: &mut HashSet<String>,
    ) -> Result<()> {
        if resolved.contains(&key.to_string()) {
            return Ok(());
        }
        if !visiting.insert(key.to_string()) {
            bail!("circular dependency detected: {key}");
        }

        let skill = self.get(key)?;
        for dep in &skill.depends {
            self.resolve_recursive(dep, resolved, visiting)?;
        }

        visiting.remove(key);
        resolved.push(key.to_string());
        Ok(())
    }
}

/// Parse a skill file, extracting optional YAML frontmatter and markdown content.
///
/// # Parsing algorithm
/// 1. Check if file starts with `---` (frontmatter delimiter)
/// 2. If yes: extract YAML between first and second `---`, parse as `Frontmatter`
/// 3. Remaining content after second `---` is the prompt body
/// 4. If no frontmatter: fall back to heading-based extraction (original behavior)
/// 5. Extract `# Heading` as name, first non-empty line as description
fn parse_skill_file(path: &Path) -> Result<Skill> {
    let raw = std::fs::read_to_string(path)?;
    let key = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    // Try to extract YAML frontmatter
    let (frontmatter, body) = extract_frontmatter(&raw);

    // Parse the body for heading and description
    let (name, description, prompt_content) = parse_body(&body, &key);

    // Frontmatter overrides heading-based values when present
    let fm = frontmatter.unwrap_or_default();

    Ok(Skill {
        key,
        name,
        description: fm.description.unwrap_or(description),
        content: prompt_content,
        category: fm.category,
        tags: fm.tags,
        required_env: fm.env,
        verify_command: fm.verify,
        depends: fm.depends,
    })
}

/// Extract YAML frontmatter from a markdown file.
///
/// # How it works
/// Looks for content between two `---` lines at the start of the file.
/// Returns `(Some(Frontmatter), remaining_body)` if found, or
/// `(None, full_content)` if no frontmatter present.
///
/// # Why not use a crate?
/// Frontmatter extraction is simple enough that a dedicated crate would be
/// over-engineering. The logic is: find `---`, find next `---`, YAML is between them.
fn extract_frontmatter(content: &str) -> (Option<Frontmatter>, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find the closing `---` (skip the opening one)
    let after_opening = &trimmed[3..];
    let after_opening = after_opening.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_opening.find("\n---") {
        let yaml_str = &after_opening[..end_pos];
        let rest = &after_opening[end_pos + 4..]; // skip "\n---"
        let rest = rest.trim_start_matches(['\r', '\n']);

        match serde_yaml::from_str::<Frontmatter>(yaml_str) {
            Ok(fm) => (Some(fm), rest.to_string()),
            Err(e) => {
                tracing::warn!("invalid skill frontmatter: {e}");
                (None, content.to_string())
            }
        }
    } else {
        (None, content.to_string())
    }
}

/// Parse the markdown body for heading, description, and prompt content.
///
/// Returns `(name, description, prompt_content)`.
fn parse_body(body: &str, default_name: &str) -> (String, String, String) {
    let mut name = default_name.to_string();
    let mut description = String::new();
    let mut prompt_content = String::new();
    let mut past_heading = false;

    for line in body.lines() {
        if !past_heading {
            if let Some(heading) = line.strip_prefix("# ") {
                name = heading.trim().to_string();
                past_heading = true;
                continue;
            }
        }

        if past_heading {
            if description.is_empty() && !line.trim().is_empty() {
                description = line.trim().to_string();
            }
            prompt_content.push_str(line);
            prompt_content.push('\n');
        }
    }

    // If no heading found, use entire content
    if !past_heading {
        prompt_content = body.to_string();
        if let Some(first_line) = body.lines().find(|l| !l.trim().is_empty()) {
            description = first_line.trim().to_string();
        }
    }

    (name, description, prompt_content.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scan_empty_dir() {
        let dir = TempDir::new().unwrap();
        let loader = SkillLoader::new(dir.path());
        assert!(loader.scan().is_empty());
    }

    #[test]
    fn scan_finds_skills() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("pr-review.md"),
            "# PR Review\n\nReview the PR changes.\n- Check for bugs\n",
        )
        .unwrap();
        std::fs::write(
            skills_dir.join("refactor.md"),
            "# Refactor\n\nRefactor the code.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skills = loader.scan();
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].key, "pr-review");
        assert_eq!(skills[0].name, "PR Review");
        assert!(skills[0].description.contains("Review the PR"));
        assert_eq!(skills[1].key, "refactor");
    }

    #[test]
    fn get_specific_skill() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("test.md"),
            "# Test Skill\n\nDo testing things.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("test").unwrap();
        assert_eq!(skill.name, "Test Skill");
        assert!(skill.content.contains("Do testing things"));
    }

    #[test]
    fn get_missing_skill_fails() {
        let dir = TempDir::new().unwrap();
        let loader = SkillLoader::new(dir.path());
        assert!(loader.get("nonexistent").is_err());
    }

    #[test]
    fn skill_without_heading() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("plain.md"),
            "Just some instructions.\nDo this and that.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("plain").unwrap();
        assert_eq!(skill.key, "plain");
        assert_eq!(skill.name, "plain");
        assert!(skill.content.contains("Just some instructions"));
    }

    #[test]
    fn parse_yaml_frontmatter() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("docker.md"),
            r#"---
description: "Manage Docker containers"
category: infrastructure
tags: [docker, containers]
env:
  - DOCKER_HOST
  - DOCKER_CONFIG
verify: "docker info"
---
# Docker Management

Use docker commands to manage containers.
"#,
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("docker").unwrap();

        assert_eq!(skill.name, "Docker Management");
        assert_eq!(skill.description, "Manage Docker containers");
        assert_eq!(skill.category, Some("infrastructure".to_string()));
        assert_eq!(skill.tags, vec!["docker", "containers"]);
        assert_eq!(skill.required_env, vec!["DOCKER_HOST", "DOCKER_CONFIG"]);
        assert_eq!(skill.verify_command, Some("docker info".to_string()));
        assert!(skill.content.contains("Use docker commands"));
    }

    #[test]
    fn frontmatter_description_overrides_heading() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("test.md"),
            "---\ndescription: \"Custom description\"\n---\n# Heading\n\nBody text here.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("test").unwrap();
        assert_eq!(skill.description, "Custom description");
        assert_eq!(skill.name, "Heading");
    }

    #[test]
    fn partial_frontmatter_works() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("minimal.md"),
            "---\ncategory: meta\n---\n# Minimal\n\nJust a category.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("minimal").unwrap();
        assert_eq!(skill.category, Some("meta".to_string()));
        assert!(skill.tags.is_empty());
        assert!(skill.required_env.is_empty());
        assert!(skill.verify_command.is_none());
    }

    #[test]
    fn invalid_frontmatter_falls_back() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("bad.md"),
            "---\n[invalid yaml{{\n---\n# Still Works\n\nContent here.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("bad").unwrap();
        // Invalid YAML falls back to full-content parsing, which finds the heading
        assert_eq!(skill.name, "Still Works");
        assert!(skill.category.is_none());
        assert!(skill.content.contains("Content here"));
    }

    #[test]
    fn resolve_dependencies_transitive() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("a.md"),
            "---\ndepends: [b]\n---\n# A\n\nSkill A.\n",
        )
        .unwrap();
        std::fs::write(
            skills_dir.join("b.md"),
            "---\ndepends: [c]\n---\n# B\n\nSkill B.\n",
        )
        .unwrap();
        std::fs::write(skills_dir.join("c.md"), "# C\n\nSkill C.\n").unwrap();

        let loader = SkillLoader::new(dir.path());
        let deps = loader.resolve_dependencies("a").unwrap();
        assert_eq!(deps, vec!["c", "b", "a"]);
    }

    #[test]
    fn resolve_dependencies_circular_detected() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("x.md"),
            "---\ndepends: [y]\n---\n# X\n\nSkill X.\n",
        )
        .unwrap();
        std::fs::write(
            skills_dir.join("y.md"),
            "---\ndepends: [x]\n---\n# Y\n\nSkill Y.\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let result = loader.resolve_dependencies("x");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("circular"), "got: {err}");
    }

    #[test]
    fn resolve_dependencies_no_deps() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        std::fs::write(skills_dir.join("solo.md"), "# Solo\n\nNo deps.\n").unwrap();

        let loader = SkillLoader::new(dir.path());
        let deps = loader.resolve_dependencies("solo").unwrap();
        assert_eq!(deps, vec!["solo"]);
    }

    #[test]
    fn backward_compatible_no_frontmatter() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".anvil").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();

        // Old-style skill with no frontmatter
        std::fs::write(
            skills_dir.join("legacy.md"),
            "# Legacy Skill\n\nThis is the old format.\n- Step 1\n- Step 2\n",
        )
        .unwrap();

        let loader = SkillLoader::new(dir.path());
        let skill = loader.get("legacy").unwrap();
        assert_eq!(skill.name, "Legacy Skill");
        assert_eq!(skill.description, "This is the old format.");
        assert!(skill.category.is_none());
        assert!(skill.tags.is_empty());
        assert!(skill.required_env.is_empty());
    }
}
