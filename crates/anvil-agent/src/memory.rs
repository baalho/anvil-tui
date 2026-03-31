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

/// A single memory entry.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub filename: String,
    pub content: String,
}

impl MemoryStore {
    /// Create a new memory store for the given `.anvil/memory/` directory.
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }

    /// Load all memory entries from disk.
    pub fn load_all(&self) -> Vec<MemoryEntry> {
        if !self.memory_dir.exists() {
            return Vec::new();
        }

        let mut entries = Vec::new();
        if let Ok(dir) = fs::read_dir(&self.memory_dir) {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Some(filename) = path.file_name() {
                            entries.push(MemoryEntry {
                                filename: filename.to_string_lossy().to_string(),
                                content,
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
    pub fn add(&self, content: &str) -> Result<String> {
        fs::create_dir_all(&self.memory_dir)?;

        let id = uuid::Uuid::new_v4();
        let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let filename = format!("{timestamp}-{}.md", &id.to_string()[..8]);
        let path = self.memory_dir.join(&filename);

        fs::write(&path, content)?;
        Ok(filename)
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
    pub fn as_prompt_section(&self) -> Option<String> {
        let entries = self.load_all();
        if entries.is_empty() {
            return None;
        }

        let mut section = String::from("## Project Memory\n\n");
        for entry in &entries {
            section.push_str(&entry.content);
            section.push_str("\n\n");
        }

        Some(section)
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
    fn as_prompt_section_combines_entries() {
        let (_dir, store) = setup();
        store.add("Use cargo fmt").unwrap();
        store.add("Run clippy").unwrap();

        let section = store.as_prompt_section().unwrap();
        assert!(section.contains("Project Memory"));
        assert!(section.contains("Use cargo fmt"));
        assert!(section.contains("Run clippy"));
    }

    #[test]
    fn clear_on_empty_returns_zero() {
        let (_dir, store) = setup();
        assert_eq!(store.clear().unwrap(), 0);
    }
}
