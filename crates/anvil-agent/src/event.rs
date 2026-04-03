//! Source-agnostic event abstraction for the agent loop.
//!
//! # Why this exists
//! The agent doesn't care where a prompt comes from — stdin, a file watcher,
//! a git hook, or (in v2.0) a Unix domain socket. This module defines the
//! `Event` enum that decouples trigger sources from agent logic.
//!
//! # v2.0 upgrade path
//! Adding a UDS listener means adding a new event producer that sends
//! `Event::UserPrompt` into the same channel. The dispatch loop and
//! agent logic remain unchanged.

use std::path::PathBuf;

/// A trigger that causes the agent to act.
///
/// Each variant carries the data needed to process it. Events that expect
/// a response carry a `reply_tx` channel so the agent's output routes back
/// to the correct consumer without knowing the topology.
///
/// Uses enum dispatch (not trait objects) — the compiler verifies exhaustiveness
/// and there's no dynamic dispatch overhead.
#[derive(Debug)]
pub enum Event {
    /// User typed a prompt (interactive mode, `anvil run`, or future IPC).
    UserPrompt {
        /// The user's input text.
        text: String,
        /// Session ID to resume, if any. None = use current session.
        session_id: Option<String>,
    },

    /// Files changed on disk (file watcher or git hook).
    FileChanged {
        /// Paths that were modified (already debounced and filtered).
        paths: Vec<PathBuf>,
    },

    /// Graceful shutdown request (Ctrl+C, SIGTERM, or daemon stop).
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_is_send() {
        // Events must be Send to cross tokio::spawn boundaries
        fn assert_send<T: Send>() {}
        assert_send::<Event>();
    }

    #[test]
    fn user_prompt_carries_text() {
        let event = Event::UserPrompt {
            text: "hello".into(),
            session_id: None,
        };
        match event {
            Event::UserPrompt { text, .. } => assert_eq!(text, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn file_changed_carries_paths() {
        let event = Event::FileChanged {
            paths: vec![PathBuf::from("src/main.rs")],
        };
        match event {
            Event::FileChanged { paths } => {
                assert_eq!(paths.len(), 1);
                assert_eq!(paths[0], PathBuf::from("src/main.rs"));
            }
            _ => panic!("wrong variant"),
        }
    }
}
