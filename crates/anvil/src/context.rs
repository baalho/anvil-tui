//! Context file system — overflow output to `.anvil/context/` files.
//!
//! When tool output exceeds a threshold, the full output is saved to a
//! timestamped file in `.anvil/context/`. The terminal shows truncated
//! output with a reference to the full file. This replaces the Zellij
//! pane integration with something editor-agnostic.
//!
//! Files are named `<tool>-<timestamp>.txt` and can be opened in any
//! editor, pager, or terminal multiplexer.

use anyhow::Result;
use std::path::{Path, PathBuf};

/// Minimum line count before overflow kicks in.
const OVERFLOW_THRESHOLD: usize = 100;

/// Context directory relative to workspace.
const CONTEXT_DIR: &str = ".anvil/context";

/// Save overflow output to a context file.
///
/// Returns the path to the saved file if the output exceeded the threshold,
/// or `None` if the output was short enough to display inline.
pub fn save_overflow(workspace: &Path, tool_name: &str, content: &str) -> Result<Option<PathBuf>> {
    if content.lines().count() <= OVERFLOW_THRESHOLD {
        return Ok(None);
    }

    let dir = workspace.join(CONTEXT_DIR);
    std::fs::create_dir_all(&dir)?;

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let filename = format!("{tool_name}-{timestamp}.txt");
    let path = dir.join(&filename);

    std::fs::write(&path, content)?;

    Ok(Some(path))
}

/// List all context files in the workspace, sorted by modification time (newest first).
pub fn list_context_files(workspace: &Path) -> Result<Vec<ContextFile>> {
    let dir = workspace.join(CONTEXT_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files: Vec<ContextFile> = std::fs::read_dir(&dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let modified = metadata.modified().ok()?;
            let size = metadata.len();
            Some(ContextFile {
                name,
                path: entry.path(),
                modified,
                size,
            })
        })
        .collect();

    files.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(files)
}

/// Read a context file by name (exact match or prefix match).
pub fn read_context_file(workspace: &Path, name: &str) -> Result<String> {
    let dir = workspace.join(CONTEXT_DIR);

    // Try exact match first
    let exact = dir.join(name);
    if exact.exists() {
        return Ok(std::fs::read_to_string(&exact)?);
    }

    // Try prefix match
    let files = list_context_files(workspace)?;
    let matches: Vec<&ContextFile> = files.iter().filter(|f| f.name.starts_with(name)).collect();

    match matches.len() {
        0 => anyhow::bail!("no context file matching '{name}'"),
        1 => Ok(std::fs::read_to_string(&matches[0].path)?),
        n => anyhow::bail!("ambiguous: '{name}' matches {n} files. Be more specific."),
    }
}

/// Clean up old context files, keeping only the most recent `keep` files.
pub fn cleanup_context(workspace: &Path, keep: usize) -> Result<usize> {
    let files = list_context_files(workspace)?;
    let mut removed = 0;
    for file in files.iter().skip(keep) {
        if std::fs::remove_file(&file.path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

/// A context file entry.
pub struct ContextFile {
    pub name: String,
    pub path: PathBuf,
    pub modified: std::time::SystemTime,
    pub size: u64,
}

impl ContextFile {
    /// Format the file size for display.
    pub fn size_display(&self) -> String {
        if self.size >= 1024 * 1024 {
            format!("{:.1}M", self.size as f64 / (1024.0 * 1024.0))
        } else if self.size >= 1024 {
            format!("{:.1}K", self.size as f64 / 1024.0)
        } else {
            format!("{}B", self.size)
        }
    }

    /// Format the modification time as a relative string.
    pub fn age_display(&self) -> String {
        let Ok(elapsed) = self.modified.elapsed() else {
            return "unknown".to_string();
        };
        let secs = elapsed.as_secs();
        if secs < 60 {
            "just now".to_string()
        } else if secs < 3600 {
            format!("{} min ago", secs / 60)
        } else if secs < 86400 {
            format!("{} hours ago", secs / 3600)
        } else {
            format!("{} days ago", secs / 86400)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_overflow_short_content_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = save_overflow(dir.path(), "shell", "short output").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn save_overflow_long_content_creates_file() {
        let dir = TempDir::new().unwrap();
        let long_content: String = (0..150).map(|i| format!("line {i}\n")).collect();
        let result = save_overflow(dir.path(), "shell", &long_content).unwrap();
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("shell-"));
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, long_content);
    }

    #[test]
    fn list_and_read_context_files() {
        let dir = TempDir::new().unwrap();
        let ctx_dir = dir.path().join(CONTEXT_DIR);
        std::fs::create_dir_all(&ctx_dir).unwrap();

        std::fs::write(ctx_dir.join("shell-20260405-1234.txt"), "content1").unwrap();
        std::fs::write(ctx_dir.join("grep-20260405-1235.txt"), "content2").unwrap();

        let files = list_context_files(dir.path()).unwrap();
        assert_eq!(files.len(), 2);

        let content = read_context_file(dir.path(), "shell-20260405-1234.txt").unwrap();
        assert_eq!(content, "content1");

        // Prefix match
        let content = read_context_file(dir.path(), "grep-").unwrap();
        assert_eq!(content, "content2");
    }

    #[test]
    fn cleanup_keeps_recent() {
        let dir = TempDir::new().unwrap();
        let ctx_dir = dir.path().join(CONTEXT_DIR);
        std::fs::create_dir_all(&ctx_dir).unwrap();

        for i in 0..5 {
            std::fs::write(
                ctx_dir.join(format!("file-{i}.txt")),
                format!("content {i}"),
            )
            .unwrap();
            // Small delay so mtime differs
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let removed = cleanup_context(dir.path(), 2).unwrap();
        assert_eq!(removed, 3);

        let remaining = list_context_files(dir.path()).unwrap();
        assert_eq!(remaining.len(), 2);
    }
}
