//! Configuration migration — upgrades old `.anvil/config.toml` formats in place.
//!
//! # How it works
//! Each migration is a function that checks for old format patterns and
//! transforms them to the current format. Migrations run in order and are
//! idempotent — running them on an already-migrated config is a no-op.

use anyhow::Result;
use std::path::Path;

/// A single config migration step.
struct Migration {
    /// Version this migration upgrades from.
    from_version: &'static str,
    /// Description logged when migration runs.
    description: &'static str,
    /// Transform function: takes old TOML string, returns new TOML string.
    /// Returns None if no migration needed.
    transform: fn(&str) -> Option<String>,
}

/// All registered migrations, in chronological order.
const MIGRATIONS: &[Migration] = &[
    Migration {
        from_version: "0.1.0",
        description: "rename 'output_limit' to 'tool.output_limit'",
        transform: migrate_output_limit,
    },
    Migration {
        from_version: "0.1.0",
        description: "add auto_compact_threshold default",
        transform: migrate_auto_compact,
    },
];

/// Run all migrations on a config file. Returns list of applied migration descriptions.
pub fn migrate_config(config_path: &Path) -> Result<Vec<String>> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let mut content = std::fs::read_to_string(config_path)?;
    let mut applied = Vec::new();

    for migration in MIGRATIONS {
        if let Some(new_content) = (migration.transform)(&content) {
            content = new_content;
            applied.push(format!(
                "{}: {}",
                migration.from_version, migration.description
            ));
        }
    }

    if !applied.is_empty() {
        std::fs::write(config_path, &content)?;
    }

    Ok(applied)
}

/// Migrate bare `output_limit = N` to `[tool]\noutput_limit = N`.
fn migrate_output_limit(content: &str) -> Option<String> {
    // Only migrate if there's a bare output_limit not under a [tool] section
    if !content.contains("output_limit") {
        return None;
    }
    if content.contains("[tool]") {
        return None;
    }

    // Find and move the bare key
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut output_limit_line = None;
    let mut idx = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("output_limit") && trimmed.contains('=') {
            output_limit_line = Some(line.clone());
            idx = Some(i);
            break;
        }
    }

    if let (Some(line), Some(i)) = (output_limit_line, idx) {
        lines.remove(i);
        lines.push(String::new());
        lines.push("[tool]".to_string());
        lines.push(line);
        Some(lines.join("\n"))
    } else {
        None
    }
}

/// Add auto_compact_threshold if missing from [agent] section.
fn migrate_auto_compact(content: &str) -> Option<String> {
    if content.contains("auto_compact_threshold") {
        return None;
    }
    // Only add if there's an [agent] section
    if !content.contains("[agent]") {
        return None;
    }

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut agent_idx = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "[agent]" {
            agent_idx = Some(i);
            break;
        }
    }

    if let Some(i) = agent_idx {
        lines.insert(i + 1, "auto_compact_threshold = 90".to_string());
        Some(lines.join("\n"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn migrate_bare_output_limit() {
        let result = migrate_output_limit("output_limit = 50000\nmodel = \"qwen3\"");
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("[tool]"));
        assert!(content.contains("output_limit = 50000"));
    }

    #[test]
    fn skip_already_migrated_output_limit() {
        let content = "[tool]\noutput_limit = 50000";
        assert!(migrate_output_limit(content).is_none());
    }

    #[test]
    fn migrate_adds_auto_compact() {
        let content = "[agent]\nloop_limit = 5";
        let result = migrate_auto_compact(content);
        assert!(result.is_some());
        assert!(result.unwrap().contains("auto_compact_threshold = 90"));
    }

    #[test]
    fn skip_auto_compact_if_present() {
        let content = "[agent]\nauto_compact_threshold = 80";
        assert!(migrate_auto_compact(content).is_none());
    }

    #[test]
    fn migrate_config_file() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "[agent]\nloop_limit = 5\n").unwrap();

        let applied = migrate_config(&config).unwrap();
        assert!(!applied.is_empty());

        let content = std::fs::read_to_string(&config).unwrap();
        assert!(content.contains("auto_compact_threshold"));
    }

    #[test]
    fn migrate_nonexistent_file_is_noop() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("missing.toml");
        let applied = migrate_config(&config).unwrap();
        assert!(applied.is_empty());
    }
}
