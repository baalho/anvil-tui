//! File watcher — converts filesystem events into agent `Event`s.
//!
//! # Why this exists
//! `anvil watch` monitors the workspace for file changes and triggers
//! agent turns automatically. This replaces the need for external tools
//! like `watchexec` or `entr`.
//!
//! # Debounce strategy
//! Editors save files in multiple steps (write temp, rename, chmod).
//! We collect events into a batch and wait for a quiet period before
//! firing. The debounce window is configurable (default: 2 seconds).
//!
//! # v2.0 upgrade path
//! This exact code runs in the daemon's background. The only change is
//! that `cmd_watch` becomes `daemon_watch` — the watcher logic is identical.

use anvil_agent::Event;
use anvil_tools::WriteLedger;
use anyhow::{bail, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Paths that are always ignored regardless of user config.
const ALWAYS_IGNORE: &[&str] = &[
    "/.git/",
    "/target/",
    "/node_modules/",
    "/__pycache__/",
    "/.anvil/achievements",
    "/.anvil/memory/",
    "/.anvil/sessions",
];

/// File extensions that are always ignored (editor artifacts).
const NOISE_EXTENSIONS: &[&str] = &[
    "swp", "swo", "swn", // vim swap
    "tmp", "bak", // generic temp
    "pyc", "pyo", // python bytecode
    "o", "a", // compiled objects
];

/// Configuration for the file watcher.
pub struct WatchConfig {
    pub workspace: PathBuf,
    pub debounce: Duration,
    pub ignore_patterns: Vec<String>,
    /// Write ledger for suppressing the agent's own file modifications.
    /// When set, filesystem events whose mtime matches a ledger entry
    /// are silently dropped to prevent feedback loops.
    pub write_ledger: Option<WriteLedger>,
}

/// Start watching the workspace and feed events into the dispatch channel.
///
/// Blocks the current thread (runs in a `tokio::task::spawn_blocking` context).
/// Returns when the event channel is closed or the watcher errors.
pub fn run_file_watcher(config: WatchConfig, event_tx: mpsc::Sender<Event>) -> Result<()> {
    let (notify_tx, notify_rx) = std_mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        notify_tx,
        notify::Config::default().with_poll_interval(Duration::from_secs(1)),
    )?;

    watcher.watch(&config.workspace, RecursiveMode::Recursive)?;

    tracing::info!(
        "watching {} (debounce: {}s)",
        config.workspace.display(),
        config.debounce.as_secs()
    );

    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut last_event_time: Option<Instant> = None;

    loop {
        // Non-blocking check for new filesystem events
        match notify_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(notify_event)) => {
                let relevant: Vec<PathBuf> = notify_event
                    .paths
                    .into_iter()
                    .filter(|p| is_relevant(p, &config.ignore_patterns))
                    .filter(|p| {
                        // Check the write ledger — suppress the agent's own writes
                        if let Some(ref ledger) = config.write_ledger {
                            if ledger.check_and_consume(p) {
                                tracing::debug!("suppressed agent write: {}", p.display());
                                return false;
                            }
                        }
                        true
                    })
                    .collect();

                if !relevant.is_empty() {
                    for p in relevant {
                        pending.insert(p);
                    }
                    last_event_time = Some(Instant::now());
                }
            }
            Ok(Err(e)) => {
                tracing::warn!("watcher error: {e}");
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                // Fall through to debounce check
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => {
                bail!("filesystem watcher disconnected");
            }
        }

        // Fire if we have pending events and debounce period elapsed
        if let Some(last) = last_event_time {
            if last.elapsed() >= config.debounce && !pending.is_empty() {
                let paths: Vec<PathBuf> = pending.drain().collect();
                last_event_time = None;

                if event_tx
                    .blocking_send(Event::FileChanged { paths })
                    .is_err()
                {
                    tracing::info!("dispatch channel closed, stopping watcher");
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Check if a path is relevant (not noise, not ignored).
fn is_relevant(path: &Path, user_ignores: &[String]) -> bool {
    let path_str = path.to_string_lossy();

    // Always-ignore paths
    for pattern in ALWAYS_IGNORE {
        if path_str.contains(pattern) {
            return false;
        }
    }

    // Noise extensions
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if NOISE_EXTENSIONS.contains(&ext) {
            return false;
        }
    }

    // Hidden files (dotfiles in the changed path component)
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') && name != ".env" {
            return false;
        }
    }

    // User-specified ignore patterns (simple substring match)
    for pattern in user_ignores {
        if path_str.contains(pattern.as_str()) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_git_directory() {
        assert!(!is_relevant(Path::new("/project/.git/objects/abc123"), &[]));
    }

    #[test]
    fn ignores_target_directory() {
        assert!(!is_relevant(
            Path::new("/project/target/debug/build/foo"),
            &[]
        ));
    }

    #[test]
    fn ignores_node_modules() {
        assert!(!is_relevant(
            Path::new("/project/node_modules/express/index.js"),
            &[]
        ));
    }

    #[test]
    fn ignores_vim_swap_files() {
        assert!(!is_relevant(Path::new("/project/src/main.rs.swp"), &[]));
    }

    #[test]
    fn ignores_hidden_files() {
        assert!(!is_relevant(Path::new("/project/.DS_Store"), &[]));
    }

    #[test]
    fn allows_dotenv() {
        assert!(is_relevant(Path::new("/project/.env"), &[]));
    }

    #[test]
    fn allows_normal_source_files() {
        assert!(is_relevant(Path::new("/project/src/main.rs"), &[]));
        assert!(is_relevant(Path::new("/project/lib/utils.py"), &[]));
        assert!(is_relevant(Path::new("/project/index.ts"), &[]));
    }

    #[test]
    fn user_ignore_patterns() {
        let ignores = vec!["vendor/".to_string(), "dist/".to_string()];
        assert!(!is_relevant(Path::new("/project/vendor/lib.go"), &ignores));
        assert!(!is_relevant(Path::new("/project/dist/bundle.js"), &ignores));
        assert!(is_relevant(Path::new("/project/src/app.go"), &ignores));
    }

    #[test]
    fn ignores_pycache() {
        assert!(!is_relevant(
            Path::new("/project/__pycache__/module.cpython-311.pyc"),
            &[]
        ));
    }

    #[test]
    fn ignores_pyc_extension() {
        assert!(!is_relevant(Path::new("/project/module.pyc"), &[]));
    }
}
