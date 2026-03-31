//! Achievement system — unlockable badges that celebrate coding milestones.
//!
//! # Why this exists
//! Achievements gamify the learning experience for kids (and adults who enjoy
//! a bit of fun). Badges are unlocked by actions like running first command,
//! fixing a bug, or writing a file. They persist across sessions in a JSON file.
//!
//! # Storage
//! Achievements are stored in `.anvil/achievements.json`. Each entry records
//! the badge key, unlock timestamp, and which persona was active (if any).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A badge definition.
#[derive(Debug, Clone)]
pub struct Badge {
    /// Unique identifier (e.g., "first_command").
    pub key: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Description of how to earn it.
    pub description: &'static str,
    /// Emoji icon.
    pub icon: &'static str,
}

/// All available badges.
pub const BADGES: &[Badge] = &[
    Badge {
        key: "first_command",
        name: "Hello World",
        icon: "🌟",
        description: "Run your first shell command",
    },
    Badge {
        key: "first_file",
        name: "File Creator",
        icon: "📝",
        description: "Write your first file",
    },
    Badge {
        key: "bug_squasher",
        name: "Bug Squasher",
        icon: "🐛",
        description: "Fix a failing test or command",
    },
    Badge {
        key: "explorer",
        name: "Code Explorer",
        icon: "🔍",
        description: "Read 10 different files in one session",
    },
    Badge {
        key: "git_hero",
        name: "Git Hero",
        icon: "🦸",
        description: "Make your first git commit",
    },
    Badge {
        key: "tool_master",
        name: "Tool Master",
        icon: "🔧",
        description: "Use 5 different tools in one session",
    },
    Badge {
        key: "marathon",
        name: "Marathon Coder",
        icon: "🏃",
        description: "Have a session with 20+ messages",
    },
    Badge {
        key: "memory_keeper",
        name: "Memory Keeper",
        icon: "🧠",
        description: "Save your first project memory",
    },
    Badge {
        key: "persona_fan",
        name: "Persona Fan",
        icon: "🎭",
        description: "Activate a character persona",
    },
    Badge {
        key: "skill_user",
        name: "Skill User",
        icon: "📚",
        description: "Activate a skill",
    },
];

/// A single unlocked achievement record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockedBadge {
    pub key: String,
    pub unlocked_at: String,
    pub persona: Option<String>,
}

/// Persistent achievement store backed by a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AchievementStore {
    pub unlocked: Vec<UnlockedBadge>,
    #[serde(skip)]
    path: PathBuf,
}

impl AchievementStore {
    /// Load achievements from disk, or create an empty store.
    pub fn load(workspace: &Path) -> Self {
        let path = workspace.join(".anvil/achievements.json");
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(mut store) = serde_json::from_str::<AchievementStore>(&data) {
                store.path = path;
                return store;
            }
        }
        AchievementStore {
            unlocked: Vec::new(),
            path,
        }
    }

    /// Save achievements to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    /// Check if a badge has been unlocked.
    pub fn is_unlocked(&self, key: &str) -> bool {
        self.unlocked.iter().any(|b| b.key == key)
    }

    /// Unlock a badge. Returns the badge definition if newly unlocked, None if already earned.
    pub fn unlock(&mut self, key: &str, persona: Option<&str>) -> Option<&'static Badge> {
        if self.is_unlocked(key) {
            return None;
        }

        let badge = BADGES.iter().find(|b| b.key == key)?;

        self.unlocked.push(UnlockedBadge {
            key: key.to_string(),
            unlocked_at: chrono::Utc::now().to_rfc3339(),
            persona: persona.map(|s| s.to_string()),
        });

        let _ = self.save();
        Some(badge)
    }

    /// Get all unlocked badge keys as a set.
    pub fn unlocked_keys(&self) -> HashSet<String> {
        self.unlocked.iter().map(|b| b.key.clone()).collect()
    }

    /// Count of unlocked badges.
    pub fn count(&self) -> usize {
        self.unlocked.len()
    }

    /// Total available badges.
    pub fn total() -> usize {
        BADGES.len()
    }

    /// Format a badge unlock notification, optionally themed by persona.
    pub fn format_unlock(badge: &Badge, persona: Option<&str>) -> String {
        let themed = match persona {
            Some("sparkle") => format!(
                "✨ MAGIC ACHIEVEMENT UNLOCKED! ✨\n  {} {} — {}\n  You're amazing!",
                badge.icon, badge.name, badge.description
            ),
            Some("bolt") => format!(
                "[BEEP BOOP] ACHIEVEMENT UNLOCKED!\n  {} {} — {}\n  SYSTEMS UPGRADED!",
                badge.icon, badge.name, badge.description
            ),
            Some("codebeard") => format!(
                "⚓ TREASURE FOUND! ⚓\n  {} {} — {}\n  Arr, well done matey!",
                badge.icon, badge.name, badge.description
            ),
            _ => format!(
                "🏆 Achievement unlocked!\n  {} {} — {}",
                badge.icon, badge.name, badge.description
            ),
        };
        themed
    }
}

/// Tracks tool usage within a session for achievement detection.
#[derive(Debug, Default)]
pub struct SessionTracker {
    pub tools_used: HashSet<String>,
    pub files_read: HashSet<String>,
    pub message_count: usize,
}

impl SessionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a tool call and return any newly triggered achievement keys.
    pub fn record_tool_call(&mut self, tool_name: &str, args_json: &str) -> Vec<&'static str> {
        let mut triggers = Vec::new();

        self.tools_used.insert(tool_name.to_string());

        match tool_name {
            "shell" => triggers.push("first_command"),
            "file_write" => triggers.push("first_file"),
            "file_read" => {
                if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_json) {
                    if let Some(path) = args["path"].as_str() {
                        self.files_read.insert(path.to_string());
                    }
                }
                if self.files_read.len() >= 10 {
                    triggers.push("explorer");
                }
            }
            "git_commit" => triggers.push("git_hero"),
            _ => {}
        }

        if self.tools_used.len() >= 5 {
            triggers.push("tool_master");
        }

        triggers
    }

    /// Record a message and return any triggered achievement keys.
    pub fn record_message(&mut self) -> Vec<&'static str> {
        self.message_count += 1;
        if self.message_count >= 20 {
            vec!["marathon"]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_empty_store() {
        let dir = TempDir::new().unwrap();
        let store = AchievementStore::load(dir.path());
        assert_eq!(store.count(), 0);
        assert!(!store.is_unlocked("first_command"));
    }

    #[test]
    fn unlock_badge() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".anvil")).unwrap();
        let mut store = AchievementStore::load(dir.path());

        let badge = store.unlock("first_command", None);
        assert!(badge.is_some());
        assert_eq!(badge.unwrap().name, "Hello World");
        assert!(store.is_unlocked("first_command"));
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn unlock_duplicate_returns_none() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".anvil")).unwrap();
        let mut store = AchievementStore::load(dir.path());

        store.unlock("first_command", None);
        assert!(store.unlock("first_command", None).is_none());
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn save_and_reload() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".anvil")).unwrap();

        {
            let mut store = AchievementStore::load(dir.path());
            store.unlock("first_command", Some("sparkle"));
            store.unlock("first_file", None);
        }

        let store = AchievementStore::load(dir.path());
        assert_eq!(store.count(), 2);
        assert!(store.is_unlocked("first_command"));
        assert!(store.is_unlocked("first_file"));
        assert_eq!(store.unlocked[0].persona.as_deref(), Some("sparkle"));
    }

    #[test]
    fn format_unlock_default() {
        let badge = &BADGES[0]; // first_command
        let msg = AchievementStore::format_unlock(badge, None);
        assert!(msg.contains("Achievement unlocked"));
        assert!(msg.contains("Hello World"));
    }

    #[test]
    fn format_unlock_sparkle() {
        let badge = &BADGES[0];
        let msg = AchievementStore::format_unlock(badge, Some("sparkle"));
        assert!(msg.contains("MAGIC"));
        assert!(msg.contains("amazing"));
    }

    #[test]
    fn format_unlock_bolt() {
        let badge = &BADGES[0];
        let msg = AchievementStore::format_unlock(badge, Some("bolt"));
        assert!(msg.contains("BEEP BOOP"));
    }

    #[test]
    fn format_unlock_codebeard() {
        let badge = &BADGES[0];
        let msg = AchievementStore::format_unlock(badge, Some("codebeard"));
        assert!(msg.contains("TREASURE"));
        assert!(msg.contains("matey"));
    }

    #[test]
    fn session_tracker_shell_triggers_first_command() {
        let mut tracker = SessionTracker::new();
        let triggers = tracker.record_tool_call("shell", "{}");
        assert!(triggers.contains(&"first_command"));
    }

    #[test]
    fn session_tracker_five_tools_triggers_tool_master() {
        let mut tracker = SessionTracker::new();
        tracker.record_tool_call("shell", "{}");
        tracker.record_tool_call("file_read", "{}");
        tracker.record_tool_call("file_write", "{}");
        tracker.record_tool_call("grep", "{}");
        let triggers = tracker.record_tool_call("ls", "{}");
        assert!(triggers.contains(&"tool_master"));
    }

    #[test]
    fn session_tracker_ten_files_triggers_explorer() {
        let mut tracker = SessionTracker::new();
        for i in 0..10 {
            let args = format!("{{\"path\": \"file{i}.txt\"}}");
            let triggers = tracker.record_tool_call("file_read", &args);
            if i == 9 {
                assert!(triggers.contains(&"explorer"));
            }
        }
    }

    #[test]
    fn session_tracker_marathon() {
        let mut tracker = SessionTracker::new();
        for i in 0..20 {
            let triggers = tracker.record_message();
            if i == 19 {
                assert!(triggers.contains(&"marathon"));
            } else {
                assert!(triggers.is_empty());
            }
        }
    }

    #[test]
    fn all_badges_have_unique_keys() {
        let keys: HashSet<&str> = BADGES.iter().map(|b| b.key).collect();
        assert_eq!(keys.len(), BADGES.len());
    }

    #[test]
    fn total_badge_count() {
        assert_eq!(AchievementStore::total(), 10);
    }
}
