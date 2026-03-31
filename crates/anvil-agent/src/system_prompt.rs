use crate::memory::MemoryStore;
use crate::skills::Skill;
use std::path::Path;

const BASE_PROMPT: &str = r#"You are Anvil, a coding assistant that runs locally. You help users with programming tasks by reading, writing, and editing files, running commands, and searching code.

## Rules
- Always read a file before editing it to understand its structure.
- Use file_edit for precise changes (search/replace with exact match). Use file_write only for new files or complete rewrites.
- When running shell commands, pass the full command as a string.
- Show file paths when working with files.
- Explain what you're doing briefly before taking action.
- If a task is ambiguous, ask for clarification.
- Never expose secrets, API keys, or sensitive data.

## Available Tools
- file_read: Read file contents (with optional line range)
- file_write: Create or overwrite a file
- file_edit: Search-and-replace edit (old_str must be unique and exact)
- shell: Execute a shell command (string, e.g. "ls -la src/")
- grep: Search file contents with regex
- ls: List directory contents with file types and sizes
- find: Find files matching a glob pattern recursively
"#;

const COMPATIBILITY_FILES: &[&str] = &[
    ".goosehints",
    "AGENTS.md",
    "CLAUDE.md",
    ".cursorrules",
    ".github/copilot-instructions.md",
];

pub fn build_system_prompt(
    workspace: &Path,
    override_prompt: Option<&str>,
    model_name: &str,
    active_skills: &[Skill],
) -> String {
    let mut prompt = match override_prompt {
        Some(custom) => custom.to_string(),
        None => BASE_PROMPT.to_string(),
    };

    // Environment info
    prompt.push_str("\n## Environment\n");
    prompt.push_str(&format!(
        "- Date: {}\n",
        chrono::Utc::now().format("%Y-%m-%d")
    ));
    prompt.push_str(&format!(
        "- OS: {} ({})\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    prompt.push_str(&format!("- Working directory: {}\n", workspace.display()));
    prompt.push_str(&format!("- Model: {model_name}\n"));

    // Project context from .anvil/context.md
    let context_path = workspace.join(".anvil/context.md");
    if let Ok(context) = std::fs::read_to_string(&context_path) {
        let trimmed = context.trim();
        if !trimmed.is_empty() {
            prompt.push_str("\n## Project Context\n\n");
            prompt.push_str(trimmed);
            prompt.push('\n');
        }
    }

    // Compatibility files
    for filename in COMPATIBILITY_FILES {
        let path = workspace.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                prompt.push_str(&format!("\n## From {filename}\n\n"));
                prompt.push_str(trimmed);
                prompt.push('\n');
            }
        }
    }

    // Project memory
    let memory_dir = workspace.join(".anvil/memory");
    let memory_store = MemoryStore::new(memory_dir);
    if let Some(memory_section) = memory_store.as_prompt_section() {
        prompt.push('\n');
        prompt.push_str(&memory_section);
    }

    // Active skills
    for skill in active_skills {
        prompt.push_str(&format!("\n## Skill: {}\n\n", skill.name));
        prompt.push_str(&skill.content);
        prompt.push('\n');
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn basic_prompt_includes_environment() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path(), None, "test-model", &[]);
        assert!(prompt.contains("Anvil"));
        assert!(prompt.contains("test-model"));
        assert!(prompt.contains("OS:"));
        assert!(prompt.contains("Working directory:"));
        assert!(prompt.contains("Date:"));
    }

    #[test]
    fn override_replaces_base() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path(), Some("Custom prompt"), "m", &[]);
        assert!(prompt.starts_with("Custom prompt"));
        assert!(!prompt.contains("You are Anvil"));
    }

    #[test]
    fn loads_context_file() {
        let dir = TempDir::new().unwrap();
        let anvil_dir = dir.path().join(".anvil");
        std::fs::create_dir_all(&anvil_dir).unwrap();
        std::fs::write(anvil_dir.join("context.md"), "This is a Rust project.").unwrap();

        let prompt = build_system_prompt(dir.path(), None, "m", &[]);
        assert!(prompt.contains("This is a Rust project"));
    }

    #[test]
    fn loads_compatibility_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".goosehints"), "Use cargo test").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Agent instructions").unwrap();

        let prompt = build_system_prompt(dir.path(), None, "m", &[]);
        assert!(prompt.contains("Use cargo test"));
        assert!(prompt.contains("Agent instructions"));
        assert!(prompt.contains("From .goosehints"));
        assert!(prompt.contains("From AGENTS.md"));
    }

    #[test]
    fn includes_active_skills() {
        let dir = TempDir::new().unwrap();
        let skills = vec![Skill {
            key: "review".to_string(),
            name: "Code Review".to_string(),
            description: "Review code".to_string(),
            content: "Focus on bugs and security.".to_string(),
            category: None,
            tags: Vec::new(),
            required_env: Vec::new(),
            verify_command: None,
            depends: Vec::new(),
        }];

        let prompt = build_system_prompt(dir.path(), None, "m", &skills);
        assert!(prompt.contains("Skill: Code Review"));
        assert!(prompt.contains("Focus on bugs and security"));
    }

    #[test]
    fn missing_files_silently_skipped() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path(), None, "m", &[]);
        // Should not contain any compatibility file sections
        assert!(!prompt.contains("From .goosehints"));
        assert!(!prompt.contains("From AGENTS.md"));
    }
}
