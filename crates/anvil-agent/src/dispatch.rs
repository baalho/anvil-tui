//! Event dispatch loop — routes events to the agent regardless of source.
//!
//! # Why this exists
//! In v1.9, events come from stdin and a file watcher. In v2.0, they also
//! come from a Unix domain socket. The dispatch loop doesn't know or care
//! about the source — it pattern-matches on `Event` and calls `agent.turn()`.
//!
//! # v2.0 upgrade path
//! Adding `Event::IpcPrompt` requires one new match arm that calls the same
//! `agent.turn()`. Zero changes to this module.

use crate::agent::AgentEvent;
use crate::event::Event;
use crate::Agent;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Result of dispatching a single event.
#[derive(Debug)]
pub enum DispatchResult {
    /// Event was processed normally.
    Handled,
    /// Shutdown was requested.
    Shutdown,
}

/// Format a file-change event into a prompt the agent can reason about.
fn file_change_prompt(paths: &[PathBuf]) -> String {
    let file_list: Vec<&str> = paths
        .iter()
        .filter_map(|p| p.to_str())
        .take(20) // cap to avoid prompt explosion
        .collect();

    let count = paths.len();
    let shown = file_list.len();
    let suffix = if count > shown {
        format!("\n... and {} more files", count - shown)
    } else {
        String::new()
    };

    format!(
        "The following {} file(s) were just modified:\n{}{}\n\n\
         Review the changes. If there are obvious issues (syntax errors, \
         missing imports, broken tests), fix them. Otherwise, briefly \
         summarize what changed.",
        count,
        file_list.join("\n"),
        suffix,
    )
}

/// Dispatch a single event to the agent.
///
/// This is the core function that v2.0 inherits unchanged. The only thing
/// that changes between versions is what PRODUCES events — never what
/// CONSUMES them.
pub async fn dispatch_event(
    agent: &mut Agent,
    event: Event,
    event_tx: &mpsc::Sender<AgentEvent>,
    permission_rx: mpsc::Receiver<anvil_tools::PermissionDecision>,
    cancel: CancellationToken,
) -> Result<DispatchResult> {
    match event {
        Event::UserPrompt { text, .. } => {
            if let Err(e) = agent.turn(&text, event_tx, permission_rx, cancel).await {
                let _ = event_tx
                    .send(AgentEvent::Error(format!("turn failed: {e}")))
                    .await;
            }

            // Persist agent state after every turn
            if let Err(e) = agent.persist_snapshot() {
                tracing::warn!("failed to persist session snapshot: {e}");
            }

            Ok(DispatchResult::Handled)
        }

        Event::FileChanged { paths } => {
            let prompt = file_change_prompt(&paths);

            let _ = event_tx
                .send(AgentEvent::ContentDelta(format!(
                    "\n  ⚡ {} file(s) changed, reviewing...\n",
                    paths.len()
                )))
                .await;

            if let Err(e) = agent.turn(&prompt, event_tx, permission_rx, cancel).await {
                let _ = event_tx
                    .send(AgentEvent::Error(format!("watch turn failed: {e}")))
                    .await;
            }

            if let Err(e) = agent.persist_snapshot() {
                tracing::warn!("failed to persist session snapshot: {e}");
            }

            Ok(DispatchResult::Handled)
        }

        Event::Shutdown => {
            tracing::info!("dispatch: shutdown requested");
            Ok(DispatchResult::Shutdown)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_change_prompt_single_file() {
        let paths = vec![PathBuf::from("src/main.rs")];
        let prompt = file_change_prompt(&paths);
        assert!(prompt.contains("1 file(s)"));
        assert!(prompt.contains("src/main.rs"));
    }

    #[test]
    fn file_change_prompt_caps_at_20() {
        let paths: Vec<PathBuf> = (0..30)
            .map(|i| PathBuf::from(format!("src/file_{i}.rs")))
            .collect();
        let prompt = file_change_prompt(&paths);
        assert!(prompt.contains("30 file(s)"));
        assert!(prompt.contains("and 10 more files"));
        // Should only list 20 files
        assert!(prompt.contains("file_19"));
        assert!(!prompt.contains("file_20"));
    }

    #[test]
    fn file_change_prompt_no_overflow_suffix() {
        let paths = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
        let prompt = file_change_prompt(&paths);
        assert!(!prompt.contains("more files"));
    }
}
