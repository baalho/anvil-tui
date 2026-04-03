//! Write ledger — tracks files modified by the agent to prevent watcher feedback loops.
//!
//! # The problem
//! When the agent's `file_write` or `file_edit` tool modifies a file, the
//! filesystem watcher sees the change and triggers another agent turn,
//! creating an infinite ping-pong loop.
//!
//! # The solution
//! After writing a file, the tool executor records the path and its new
//! `mtime` in this ledger. The watcher checks incoming events against the
//! ledger — if the file's current mtime matches the recorded mtime, the
//! event is the agent's own write and is silently dropped.
//!
//! # Thread safety
//! The ledger is shared between the tool executor (writes) and the file
//! watcher (reads). `Arc<RwLock<...>>` provides safe concurrent access.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// Shared ledger of files recently written by the agent.
///
/// Clone-friendly via `Arc`. The inner `RwLock` allows concurrent reads
/// from the watcher with exclusive writes from the tool executor.
#[derive(Debug, Clone)]
pub struct WriteLedger {
    inner: Arc<RwLock<HashMap<PathBuf, SystemTime>>>,
}

impl WriteLedger {
    /// Create a new empty ledger.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Record a file write. Called by the tool executor after `file_write`
    /// or `file_edit` completes successfully.
    pub fn record(&self, path: PathBuf, mtime: SystemTime) {
        if let Ok(mut ledger) = self.inner.write() {
            ledger.insert(path, mtime);
        }
    }

    /// Check if a filesystem event is the agent's own write and consume
    /// the entry if so.
    ///
    /// Returns `true` if the path is in the ledger AND the file's current
    /// mtime matches the recorded mtime (meaning no external modification
    /// happened after the agent's write). The entry is removed on match
    /// so subsequent real changes to the same file are not suppressed.
    ///
    /// Returns `false` if:
    /// - The path is not in the ledger (not an agent write)
    /// - The file's mtime differs from the recorded mtime (modified again
    ///   after the agent wrote it — a real change)
    /// - The file no longer exists or mtime can't be read
    pub fn check_and_consume(&self, path: &Path) -> bool {
        let mut ledger = match self.inner.write() {
            Ok(l) => l,
            Err(e) => e.into_inner(), // recover from poisoned lock
        };

        let recorded_mtime = match ledger.get(path) {
            Some(t) => *t,
            None => return false,
        };

        let current_mtime = match std::fs::metadata(path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => {
                // File gone or unreadable — remove stale entry, don't suppress
                ledger.remove(path);
                return false;
            }
        };

        if current_mtime == recorded_mtime {
            ledger.remove(path);
            true
        } else {
            // File was modified again after the agent's write — real change
            ledger.remove(path);
            false
        }
    }

    /// Number of entries in the ledger (for diagnostics).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.read().map(|l| l.len()).unwrap_or(0)
    }
}

impl Default for WriteLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn record_and_check_own_write() {
        let ledger = WriteLedger::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");

        // Write a file and record it
        std::fs::write(&path, "hello").unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ledger.record(path.clone(), mtime);

        // Should be recognized as our own write
        assert!(ledger.check_and_consume(&path));
        // Entry consumed — second check returns false
        assert!(!ledger.check_and_consume(&path));
    }

    #[test]
    fn external_modification_not_suppressed() {
        let ledger = WriteLedger::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");

        // Agent writes
        std::fs::write(&path, "hello").unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ledger.record(path.clone(), mtime);

        // External modification changes the mtime
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.write_all(b"modified externally").unwrap();
        f.flush().unwrap();
        drop(f);

        // Should NOT be suppressed — mtime differs
        assert!(!ledger.check_and_consume(&path));
    }

    #[test]
    fn unknown_path_returns_false() {
        let ledger = WriteLedger::new();
        assert!(!ledger.check_and_consume(Path::new("/nonexistent/file.txt")));
    }

    #[test]
    fn deleted_file_cleans_up_entry() {
        let ledger = WriteLedger::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");

        std::fs::write(&path, "hello").unwrap();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ledger.record(path.clone(), mtime);

        // Delete the file
        std::fs::remove_file(&path).unwrap();

        // Should return false and clean up the entry
        assert!(!ledger.check_and_consume(&path));
        assert_eq!(ledger.len(), 0);
    }

    #[test]
    fn ledger_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WriteLedger>();
    }
}
