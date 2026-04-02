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

## When to Use Tools vs Respond Directly
- If the user asks you to CREATE, WRITE, or SAVE a file → use file_write
- If the user asks you to EDIT or MODIFY a file → use file_edit
- If the user asks you to RUN or EXECUTE something → use shell
- If the user asks you to generate a program → use file_write to create the file, then shell to run it
- If the user asks for information, explanation, or creative output (ASCII art, stories, lists) → respond directly in text
- When in doubt, prefer using tools for anything that produces files or runs commands

## Available Tools
- file_read: Read file contents (with optional line range)
- file_write: Create or overwrite a file
- file_edit: Search-and-replace edit (old_str must be unique and exact)
- shell: Execute a shell command (string, e.g. "ls -la src/")
- grep: Search file contents with regex
- ls: List directory contents with file types and sizes
- find: Find files matching a glob pattern recursively
- git_status: Show working tree status
- git_diff: Show changes between commits/staging/working tree
- git_log: Show recent commit history
- git_commit: Stage files and create a git commit
"#;

const COMPATIBILITY_FILES: &[&str] = &[
    ".goosehints",
    "AGENTS.md",
    "CLAUDE.md",
    ".cursorrules",
    ".github/copilot-instructions.md",
];

/// Build the system prompt with layered construction for KV cache efficiency.
///
/// # Layer Contract
///
/// Layers MUST be ordered from most-stable to most-dynamic so the prefix
/// is maximally reusable across turns. MLX and llama-server KV caches
/// store the key-value pairs for each token position; if the prefix is
/// identical between turns, the cache is reused without recomputation.
/// Breaking this ordering is like rearranging the intake manifold on a
/// turbo engine — the flow path must go static → dynamic or you get
/// backpressure (cache misses) on every turn.
///
/// **Do not insert dynamic content above static content.**
///
/// 1. **Persona** (static per session) — character voice instructions
/// 2. **Base prompt + rules** (static) — core identity and tool list
/// 3. **Skills** (semi-static) — changes only on `/skill` activation
/// 4. **Project context** (semi-static) — context.md, AGENTS.md, etc.
/// 5. **Environment + memory** (dynamic) — date, cwd, learned patterns
///
/// The persona is injected by `Agent::rebuild_system_prompt()` as a prefix
/// before this function's output, so it's always at the very top.
/// This ordering is enforced by the `layer_contract_ordering` test.
pub fn build_system_prompt(
    workspace: &Path,
    override_prompt: Option<&str>,
    model_name: &str,
    active_skills: &[Skill],
) -> String {
    let mut prompt = String::with_capacity(4096);

    // --- Layer 2: Base prompt + rules (static) ---
    match override_prompt {
        Some(custom) => prompt.push_str(custom),
        None => prompt.push_str(BASE_PROMPT),
    };

    // --- Layer 3: Active skills (semi-static) ---
    // Skills change only when the user runs /skill, so they're stable
    // across most turns and benefit from KV cache reuse.
    for skill in active_skills {
        prompt.push_str(&format!("\n## Skill: {}\n\n", skill.name));
        prompt.push_str(&skill.content);
        prompt.push('\n');
    }

    // --- Layer 4: Project context (semi-static) ---
    // These files rarely change during a session.
    let context_path = workspace.join(".anvil/context.md");
    if let Ok(context) = std::fs::read_to_string(&context_path) {
        let trimmed = context.trim();
        if !trimmed.is_empty() {
            prompt.push_str("\n## Project Context\n\n");
            prompt.push_str(trimmed);
            prompt.push('\n');
        }
    }

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

    // --- Layer 4b: Infrastructure inventory (semi-static) ---
    // Loaded from .anvil/inventory.toml. Changes rarely (when hosts are added/removed).
    let inventory = anvil_config::load_inventory(workspace);
    if let Some(inv_section) = anvil_config::inventory::inventory_as_prompt(&inventory) {
        prompt.push('\n');
        prompt.push_str(&inv_section);
    }

    // --- Layer 5: Dynamic content (changes every turn) ---
    // Environment info and memory go last so the prefix above stays stable.
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

    // Project memory (dynamic — user adds/removes between turns)
    let memory_dir = workspace.join(".anvil/memory");
    let memory_store = MemoryStore::new(memory_dir);
    if let Some(memory_section) = memory_store.as_prompt_section() {
        prompt.push('\n');
        prompt.push_str(&memory_section);
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

    #[test]
    fn layer_ordering_skills_before_context_before_environment() {
        let dir = TempDir::new().unwrap();
        let anvil_dir = dir.path().join(".anvil");
        std::fs::create_dir_all(&anvil_dir).unwrap();
        std::fs::write(anvil_dir.join("context.md"), "Project context here").unwrap();

        let skills = vec![Skill {
            key: "test".to_string(),
            name: "Test Skill".to_string(),
            description: "A test skill".to_string(),
            content: "Skill content here.".to_string(),
            category: None,
            tags: Vec::new(),
            required_env: Vec::new(),
            verify_command: None,
            depends: Vec::new(),
        }];

        let prompt = build_system_prompt(dir.path(), None, "m", &skills);

        // Verify ordering: base prompt < skills < context < environment
        let base_pos = prompt.find("You are Anvil").unwrap();
        let skill_pos = prompt.find("Skill: Test Skill").unwrap();
        let context_pos = prompt.find("Project context here").unwrap();
        let env_pos = prompt.find("## Environment").unwrap();

        assert!(
            base_pos < skill_pos,
            "base prompt should come before skills"
        );
        assert!(skill_pos < context_pos, "skills should come before context");
        assert!(
            context_pos < env_pos,
            "context should come before environment"
        );
    }

    #[test]
    fn git_tools_in_base_prompt() {
        let dir = TempDir::new().unwrap();
        let prompt = build_system_prompt(dir.path(), None, "m", &[]);
        assert!(prompt.contains("git_status"));
        assert!(prompt.contains("git_diff"));
        assert!(prompt.contains("git_log"));
        assert!(prompt.contains("git_commit"));
    }

    #[test]
    fn memory_appears_after_environment() {
        let dir = TempDir::new().unwrap();
        let memory_dir = dir.path().join(".anvil/memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(
            memory_dir.join("test.md"),
            "---\ncategory: patterns\n---\nUse cargo test",
        )
        .unwrap();

        let prompt = build_system_prompt(dir.path(), None, "m", &[]);
        let env_pos = prompt.find("## Environment").unwrap();
        if let Some(memory_pos) = prompt.find("cargo test") {
            assert!(
                memory_pos > env_pos,
                "memory should appear after environment"
            );
        }
    }

    /// Enforces the full layer contract: every layer marker must appear
    /// in strict ascending order. If a future change inserts dynamic
    /// content above static content, this test fails — protecting KV
    /// cache prefix stability.
    #[test]
    fn layer_contract_ordering() {
        let dir = TempDir::new().unwrap();
        let anvil_dir = dir.path().join(".anvil");
        std::fs::create_dir_all(&anvil_dir).unwrap();
        std::fs::write(anvil_dir.join("context.md"), "LAYER4_CONTEXT_MARKER").unwrap();

        let memory_dir = anvil_dir.join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();
        std::fs::write(
            memory_dir.join("test.md"),
            "---\ncategory: patterns\n---\nLAYER5_MEMORY_MARKER",
        )
        .unwrap();

        let skills = vec![Skill {
            key: "contract".to_string(),
            name: "Contract Test".to_string(),
            description: "test".to_string(),
            content: "LAYER3_SKILL_MARKER".to_string(),
            category: None,
            tags: Vec::new(),
            required_env: Vec::new(),
            verify_command: None,
            depends: Vec::new(),
        }];

        let prompt = build_system_prompt(dir.path(), None, "test-model", &skills);

        // Layer 2: base prompt (static)
        let layer2 = prompt.find("You are Anvil").expect("base prompt missing");
        // Layer 3: skills (semi-static)
        let layer3 = prompt
            .find("LAYER3_SKILL_MARKER")
            .expect("skill marker missing");
        // Layer 4: project context (semi-static)
        let layer4 = prompt
            .find("LAYER4_CONTEXT_MARKER")
            .expect("context marker missing");
        // Layer 5a: environment (dynamic)
        let layer5a = prompt
            .find("## Environment")
            .expect("environment section missing");
        // Layer 5b: memory (dynamic)
        let layer5b = prompt
            .find("LAYER5_MEMORY_MARKER")
            .expect("memory marker missing");

        assert!(
            layer2 < layer3,
            "Layer 2 (base) must precede Layer 3 (skills)"
        );
        assert!(
            layer3 < layer4,
            "Layer 3 (skills) must precede Layer 4 (context)"
        );
        assert!(
            layer4 < layer5a,
            "Layer 4 (context) must precede Layer 5 (environment)"
        );
        assert!(
            layer5a < layer5b,
            "Layer 5a (environment) must precede Layer 5b (memory)"
        );
    }
}
