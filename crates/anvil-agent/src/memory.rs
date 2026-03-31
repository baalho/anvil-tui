//! Project memory — persistent learned patterns stored as markdown files.
//!
//! # Why this exists
//! Users discover project-specific patterns during sessions (e.g., "always run
//! `cargo fmt` before committing"). Memory persists these across sessions so
//! the agent doesn't need to re-learn them.
//!
//! # How it works
//! Each memory is a markdown file in `.anvil/memory/`. On session start, all
//! memory files are loaded and appended to the system prompt. Users manage
//! memories via `/memory`, `/memory add`, and `/memory clear`.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Manages persistent project memory stored as markdown files.
pub struct MemoryStore {
    memory_dir: PathBuf,
}

/// Memory categories for organizing learned patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCategory {
    /// Project conventions (e.g., "always run cargo fmt")
    Convention,
    /// Known gotchas or pitfalls
    Gotcha,
    /// Reusable patterns or snippets
    Pattern,
    /// General notes
    Note,
}

impl MemoryCategory {
    /// Parse from a string tag. Case-insensitive, defaults to Note.
    pub fn from_tag(tag: &str) -> Self {
        match tag.to_lowercase().as_str() {
            "convention" | "conv" => Self::Convention,
            "gotcha" | "warning" | "warn" => Self::Gotcha,
            "pattern" | "pat" => Self::Pattern,
            _ => Self::Note,
        }
    }

    /// Display label for prompt injection.
    pub fn label(&self) -> &str {
        match self {
            Self::Convention => "Convention",
            Self::Gotcha => "Gotcha",
            Self::Pattern => "Pattern",
            Self::Note => "Note",
        }
    }
}

/// A single memory entry.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub filename: String,
    pub content: String,
    pub category: MemoryCategory,
}

impl MemoryStore {
    /// Create a new memory store for the given `.anvil/memory/` directory.
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }

    /// Load all memory entries from disk.
    ///
    /// Entries can optionally start with `[category]` on the first line
    /// (e.g., `[convention]`, `[gotcha]`). If absent, defaults to Note.
    pub fn load_all(&self) -> Vec<MemoryEntry> {
        if !self.memory_dir.exists() {
            return Vec::new();
        }

        let mut entries = Vec::new();
        if let Ok(dir) = fs::read_dir(&self.memory_dir) {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    if let Ok(raw) = fs::read_to_string(&path) {
                        if let Some(filename) = path.file_name() {
                            let (category, content) = Self::parse_category(&raw);
                            entries.push(MemoryEntry {
                                filename: filename.to_string_lossy().to_string(),
                                content,
                                category,
                            });
                        }
                    }
                }
            }
        }

        entries.sort_by(|a, b| a.filename.cmp(&b.filename));
        entries
    }

    /// Add a new memory entry. Uses UUID to avoid filename collisions.
    ///
    /// If `category` is provided, it's stored as a `[category]` tag on the first line.
    pub fn add(&self, content: &str) -> Result<String> {
        self.add_with_category(content, None)
    }

    /// Add a memory with an explicit category tag.
    pub fn add_with_category(
        &self,
        content: &str,
        category: Option<&MemoryCategory>,
    ) -> Result<String> {
        fs::create_dir_all(&self.memory_dir)?;

        let id = uuid::Uuid::new_v4();
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let filename = format!("{timestamp}-{}.md", &id.to_string()[..8]);
        let path = self.memory_dir.join(&filename);

        let file_content = match category {
            Some(cat) if *cat != MemoryCategory::Note => {
                format!("[{}]\n{}", cat.label().to_lowercase(), content)
            }
            _ => content.to_string(),
        };

        fs::write(&path, file_content)?;
        Ok(filename)
    }

    /// Delete a specific memory entry by filename.
    pub fn remove(&self, filename: &str) -> Result<bool> {
        let path = self.memory_dir.join(filename);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Search memories by keyword (case-insensitive substring match).
    pub fn search(&self, query: &str) -> Vec<MemoryEntry> {
        let query_lower = query.to_lowercase();
        self.load_all()
            .into_iter()
            .filter(|e| e.content.to_lowercase().contains(&query_lower))
            .collect()
    }

    /// Remove all memory entries.
    pub fn clear(&self) -> Result<usize> {
        if !self.memory_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        if let Ok(dir) = fs::read_dir(&self.memory_dir) {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    fs::remove_file(&path)?;
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Build a combined string of all memories for injection into the system prompt.
    ///
    /// Groups entries by category with headers for better LLM comprehension.
    pub fn as_prompt_section(&self) -> Option<String> {
        let entries = self.load_all();
        if entries.is_empty() {
            return None;
        }

        let mut section = String::from("## Project Memory\n\n");

        // Group by category for structured prompt injection
        let categories = [
            MemoryCategory::Convention,
            MemoryCategory::Gotcha,
            MemoryCategory::Pattern,
            MemoryCategory::Note,
        ];

        for cat in &categories {
            let cat_entries: Vec<_> = entries.iter().filter(|e| e.category == *cat).collect();
            if cat_entries.is_empty() {
                continue;
            }
            section.push_str(&format!("### {}\n", cat.label()));
            for entry in cat_entries {
                section.push_str(&format!("- {}\n", entry.content));
            }
            section.push('\n');
        }

        Some(section)
    }

    /// Parse an optional `[category]` tag from the first line of a memory file.
    fn parse_category(raw: &str) -> (MemoryCategory, String) {
        if let Some(first_line) = raw.lines().next() {
            let trimmed = first_line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let tag = &trimmed[1..trimmed.len() - 1];
                let content = raw[first_line.len()..].trim_start_matches('\n').to_string();
                return (MemoryCategory::from_tag(tag), content);
            }
        }
        (MemoryCategory::Note, raw.to_string())
    }

    /// The directory where memory files are stored.
    pub fn memory_dir(&self) -> &Path {
        &self.memory_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, MemoryStore) {
        let dir = TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        let store = MemoryStore::new(memory_dir);
        (dir, store)
    }

    #[test]
    fn empty_memory_returns_empty() {
        let (_dir, store) = setup();
        assert!(store.load_all().is_empty());
        assert!(store.as_prompt_section().is_none());
    }

    #[test]
    fn add_and_load_memory() {
        let (_dir, store) = setup();
        let filename = store.add("Always run tests before committing").unwrap();
        assert!(filename.ends_with(".md"));

        let entries = store.load_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "Always run tests before committing");
        assert_eq!(entries[0].category, MemoryCategory::Note);
    }

    #[test]
    fn add_with_category() {
        let (_dir, store) = setup();
        store
            .add_with_category("run cargo fmt", Some(&MemoryCategory::Convention))
            .unwrap();

        let entries = store.load_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].category, MemoryCategory::Convention);
        assert_eq!(entries[0].content, "run cargo fmt");
    }

    #[test]
    fn parse_category_from_file() {
        let (cat, content) = MemoryStore::parse_category("[convention]\nrun cargo fmt");
        assert_eq!(cat, MemoryCategory::Convention);
        assert_eq!(content, "run cargo fmt");
    }

    #[test]
    fn parse_category_missing_defaults_to_note() {
        let (cat, content) = MemoryStore::parse_category("just a note");
        assert_eq!(cat, MemoryCategory::Note);
        assert_eq!(content, "just a note");
    }

    #[test]
    fn parse_category_gotcha() {
        let (cat, _) = MemoryStore::parse_category("[gotcha]\nOllama defaults to 2048");
        assert_eq!(cat, MemoryCategory::Gotcha);
    }

    #[test]
    fn clear_removes_all() {
        let (_dir, store) = setup();
        store.add("pattern 1").unwrap();
        store.add("pattern 2").unwrap();
        assert_eq!(store.load_all().len(), 2);

        let removed = store.clear().unwrap();
        assert_eq!(removed, 2);
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn as_prompt_section_groups_by_category() {
        let (_dir, store) = setup();
        store
            .add_with_category("run cargo fmt", Some(&MemoryCategory::Convention))
            .unwrap();
        store
            .add_with_category("Ollama 2048 limit", Some(&MemoryCategory::Gotcha))
            .unwrap();
        store.add("general note").unwrap();

        let section = store.as_prompt_section().unwrap();
        assert!(section.contains("### Convention"));
        assert!(section.contains("### Gotcha"));
        assert!(section.contains("### Note"));
        assert!(section.contains("run cargo fmt"));
        assert!(section.contains("Ollama 2048 limit"));
        assert!(section.contains("general note"));
    }

    #[test]
    fn search_finds_matching() {
        let (_dir, store) = setup();
        store.add("always run cargo fmt").unwrap();
        store.add("use clippy").unwrap();
        store.add("cargo test before push").unwrap();

        let results = store.search("cargo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_case_insensitive() {
        let (_dir, store) = setup();
        store.add("Run CARGO fmt").unwrap();

        let results = store.search("cargo");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_no_results() {
        let (_dir, store) = setup();
        store.add("something").unwrap();

        let results = store.search("nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn remove_specific_entry() {
        let (_dir, store) = setup();
        let f1 = store.add("keep this").unwrap();
        let f2 = store.add("remove this").unwrap();

        assert!(store.remove(&f2).unwrap());
        let entries = store.load_all();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, f1);
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let (_dir, store) = setup();
        assert!(!store.remove("nonexistent.md").unwrap());
    }

    #[test]
    fn clear_on_empty_returns_zero() {
        let (_dir, store) = setup();
        assert_eq!(store.clear().unwrap(), 0);
    }

    #[test]
    fn category_from_tag_variants() {
        assert_eq!(
            MemoryCategory::from_tag("convention"),
            MemoryCategory::Convention
        );
        assert_eq!(MemoryCategory::from_tag("conv"), MemoryCategory::Convention);
        assert_eq!(MemoryCategory::from_tag("GOTCHA"), MemoryCategory::Gotcha);
        assert_eq!(MemoryCategory::from_tag("warning"), MemoryCategory::Gotcha);
        assert_eq!(MemoryCategory::from_tag("pattern"), MemoryCategory::Pattern);
        assert_eq!(MemoryCategory::from_tag("pat"), MemoryCategory::Pattern);
        assert_eq!(MemoryCategory::from_tag("unknown"), MemoryCategory::Note);
    }
}
