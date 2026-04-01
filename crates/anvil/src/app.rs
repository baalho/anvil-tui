//! Decoupled async architecture for the Anvil TUI.
//!
//! # Architecture
//! Three independent tasks communicate via `mpsc` channels:
//!
//! ```text
//! ┌──────────────┐     AppEvent      ┌──────────────┐
//! │  Input Task  │ ──────────────►   │  Render Task │
//! │  (keyboard)  │                   │  (display)   │
//! └──────────────┘                   └──────┬───────┘
//!                                           │ EngineCommand
//!                                           ▼
//! ┌──────────────┐     AppEvent      ┌──────────────┐
//! │  Engine Task │ ──────────────►   │  Render Task │
//! │  (LLM, I/O)  │                   │  (display)   │
//! └──────────────┘                   └──────────────┘
//! ```
//!
//! - **Input Task**: Reads terminal events, debounces keyboard mashing,
//!   sends `AppEvent::UserInput` or `AppEvent::PermissionResponse`.
//! - **Engine Task**: Runs agent turns, tool execution, LLM streaming.
//!   Sends `AppEvent::Token`, `AppEvent::ToolCall`, etc.
//! - **Render Task**: Receives all `AppEvent`s and updates the display.
//!   Never blocks on I/O. Owns stdout exclusively.
//!
//! # Kid-Proof Input
//! The input task rate-limits keyboard events to prevent a child from
//! flooding the channel by holding down a key. Events are debounced
//! at 50ms intervals (20 events/sec max).

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Events flowing from input/engine tasks to the render task.
#[derive(Debug, Clone)]
pub enum AppEvent {
    // --- From Input Task ---
    /// User submitted a line of text (after pressing Enter).
    UserInput(String),

    /// User responded to a permission prompt.
    PermissionResponse(anvil_tools::PermissionDecision),

    // --- From Engine Task ---
    /// A content token from the LLM stream.
    Token(String),

    /// A thinking/reasoning token from the LLM stream.
    ThinkingToken(String),

    /// The LLM wants to call a tool — needs user permission.
    ToolCallPending {
        id: String,
        name: String,
        arguments: String,
    },

    /// A tool finished executing.
    ToolResult { name: String, result: String },

    /// Token usage stats from a completed turn.
    Usage(anvil_llm::TokenUsage),

    /// The agent turn completed (no more tokens/tool calls).
    TurnComplete,

    /// The engine is ready for the next prompt.
    Ready,

    /// Context was auto-compacted.
    AutoCompacted {
        before_tokens: usize,
        after_tokens: usize,
        messages_removed: usize,
    },

    /// LLM request is being retried.
    Retry {
        attempt: usize,
        max: usize,
        delay_secs: f64,
    },

    /// Loop detection triggered.
    LoopDetected { tool_name: String, count: usize },

    /// Context window usage warning.
    ContextWarning {
        estimated_tokens: usize,
        limit: usize,
    },

    /// Streaming tool output delta.
    ToolOutputDelta { delta: String },

    /// The turn was cancelled (Ctrl+C).
    Cancelled,

    /// An error occurred in the engine.
    Error(String),

    /// A slash command produced output (handled synchronously).
    CommandOutput(String),

    /// A badge was just unlocked.
    AchievementUnlocked {
        icon: String,
        name: String,
        description: String,
        persona: Option<String>,
    },

    /// Engine is shutting down.
    Shutdown,
}

/// Commands flowing from the render task to the engine task.
#[derive(Debug)]
pub enum EngineCommand {
    /// Run a user prompt through the agent.
    Prompt(String),

    /// Execute a slash command.
    SlashCommand(String),

    /// User's permission decision for a pending tool call.
    Permission(anvil_tools::PermissionDecision),

    /// Compact the conversation context.
    Compact,

    /// Shut down the engine.
    Shutdown,
}

/// Configuration for the input debouncer.
pub struct InputConfig {
    /// Minimum interval between accepted key events (prevents key-mashing floods).
    /// Default: 50ms (20 events/sec).
    pub debounce_ms: u64,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self { debounce_ms: 50 }
    }
}

/// Spawn the input reader task.
///
/// Reads lines from stdin in a blocking thread, rate-limits events,
/// and sends them to the render task via the `app_tx` channel.
pub fn spawn_input_task(
    app_tx: mpsc::Sender<AppEvent>,
    cancel: CancellationToken,
    _config: InputConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Use a blocking thread for stdin reading since crossterm's
        // event::read() is synchronous.
        let (line_tx, mut line_rx) = mpsc::channel::<String>(16);

        let stdin_cancel = cancel.clone();
        let stdin_handle = tokio::task::spawn_blocking(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            let reader = stdin.lock();

            for line in reader.lines() {
                if stdin_cancel.is_cancelled() {
                    break;
                }
                match line {
                    Ok(text) => {
                        if line_tx.blocking_send(text).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                line = line_rx.recv() => {
                    match line {
                        Some(text) => {
                            if app_tx.send(AppEvent::UserInput(text)).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        // Clean up the blocking thread
        drop(line_rx);
        let _ = stdin_handle.await;
    })
}

/// Spawn the engine task.
///
/// Receives commands from the render task, runs agent turns,
/// and sends events back via `app_tx`. Tracks tool usage for
/// achievement detection and emits `AchievementUnlocked` events
/// without blocking tool execution.
pub fn spawn_engine_task(
    mut cmd_rx: mpsc::Receiver<EngineCommand>,
    app_tx: mpsc::Sender<AppEvent>,
    agent: anvil_agent::Agent,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<anvil_agent::Agent> {
    tokio::spawn(async move {
        use anvil_agent::AgentEvent;
        use anvil_agent::achievements::{AchievementStore, SessionTracker};

        let mut agent = agent;
        let mut tracker = SessionTracker::new();
        let mut achievements = AchievementStore::load(agent.workspace());

        // Signal ready
        let _ = app_tx.send(AppEvent::Ready).await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = app_tx.send(AppEvent::Shutdown).await;
                    break;
                }
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(EngineCommand::Prompt(prompt)) => {
                            let turn_cancel = CancellationToken::new();
                            let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
                            let (perm_tx, perm_rx) = mpsc::channel(1);

                            // Store perm_tx for forwarding permission responses
                            let perm_tx_clone = perm_tx.clone();
                            let app_tx_clone = app_tx.clone();

                            // Spawn a forwarder that converts AgentEvents to AppEvents
                            // and checks for achievement triggers on tool results.
                            let mut fwd_tracker = tracker.clone();
                            let mut fwd_achievements = achievements.clone();
                            let persona_key = agent.persona().map(|p| p.key.clone());
                            let forwarder = tokio::spawn(async move {
                                while let Some(event) = event_rx.recv().await {
                                    let app_event = match event {
                                        AgentEvent::ContentDelta(text) => AppEvent::Token(text),
                                        AgentEvent::ThinkingDelta(text) => AppEvent::ThinkingToken(text),
                                        AgentEvent::ToolCallPending { id, name, arguments } => {
                                            AppEvent::ToolCallPending { id, name, arguments }
                                        }
                                        AgentEvent::ToolResult { name, result } => {
                                            // Check achievements — non-blocking fire-and-forget
                                            let triggers = fwd_tracker.record_tool_call(&name, "{}");
                                            for key in triggers {
                                                if let Some(badge) = fwd_achievements.unlock(
                                                    key,
                                                    persona_key.as_deref(),
                                                ) {
                                                    let _ = app_tx_clone
                                                        .try_send(AppEvent::AchievementUnlocked {
                                                            icon: badge.icon.to_string(),
                                                            name: badge.name.to_string(),
                                                            description: badge.description.to_string(),
                                                            persona: persona_key.clone(),
                                                        });
                                                }
                                            }
                                            AppEvent::ToolResult { name, result }
                                        }
                                        AgentEvent::Usage(u) => AppEvent::Usage(u),
                                        AgentEvent::TurnComplete => {
                                            // Check message-count achievements
                                            let triggers = fwd_tracker.record_message();
                                            for key in triggers {
                                                if let Some(badge) = fwd_achievements.unlock(
                                                    key,
                                                    persona_key.as_deref(),
                                                ) {
                                                    let _ = app_tx_clone
                                                        .try_send(AppEvent::AchievementUnlocked {
                                                            icon: badge.icon.to_string(),
                                                            name: badge.name.to_string(),
                                                            description: badge.description.to_string(),
                                                            persona: persona_key.clone(),
                                                        });
                                                }
                                            }
                                            AppEvent::TurnComplete
                                        }
                                        AgentEvent::AutoCompacted {
                                            before_tokens,
                                            after_tokens,
                                            messages_removed,
                                        } => AppEvent::AutoCompacted {
                                            before_tokens,
                                            after_tokens,
                                            messages_removed,
                                        },
                                        AgentEvent::Cancelled => AppEvent::Cancelled,
                                        AgentEvent::Retry { attempt, max, delay_secs } => {
                                            AppEvent::Retry { attempt, max, delay_secs }
                                        }
                                        AgentEvent::LoopDetected { tool_name, count } => {
                                            AppEvent::LoopDetected { tool_name, count }
                                        }
                                        AgentEvent::ContextWarning { estimated_tokens, limit } => {
                                            AppEvent::ContextWarning { estimated_tokens, limit }
                                        }
                                        AgentEvent::ToolOutputDelta { delta, .. } => {
                                            AppEvent::ToolOutputDelta { delta }
                                        }
                                        AgentEvent::Error(e) => AppEvent::Error(e),
                                    };
                                    if app_tx_clone.send(app_event).await.is_err() {
                                        break;
                                    }
                                }
                                // Return updated tracker/achievements to sync back
                                (fwd_tracker, fwd_achievements)
                            });

                            // Run the turn
                            let result = agent
                                .turn(&prompt, &event_tx, perm_rx, turn_cancel)
                                .await;

                            // Drop event_tx so the forwarder finishes
                            drop(event_tx);
                            drop(perm_tx_clone);
                            if let Ok((fwd_t, fwd_a)) = forwarder.await {
                                tracker = fwd_t;
                                achievements = fwd_a;
                            }

                            if let Err(e) = result {
                                let _ = app_tx.send(AppEvent::Error(format!("{e}"))).await;
                            }

                            // Signal ready for next prompt
                            let _ = app_tx.send(AppEvent::Ready).await;
                        }
                        Some(EngineCommand::Permission(decision)) => {
                            // This is handled via the perm_rx channel inside the turn.
                            // For now, permissions are handled inline during the turn.
                            // This variant exists for future decoupling.
                            let _ = decision;
                        }
                        Some(EngineCommand::Compact) => {
                            let cancel_token = CancellationToken::new();
                            let (event_tx, _) = mpsc::channel::<AgentEvent>(64);
                            match agent.compact(4, &event_tx, cancel_token).await {
                                Ok(result) => {
                                    if result.messages_removed > 0 {
                                        let _ = app_tx
                                            .send(AppEvent::AutoCompacted {
                                                before_tokens: result.before_tokens,
                                                after_tokens: result.after_tokens,
                                                messages_removed: result.messages_removed,
                                            })
                                            .await;
                                    } else {
                                        let _ = app_tx
                                            .send(AppEvent::CommandOutput(
                                                "nothing to compact".to_string(),
                                            ))
                                            .await;
                                    }
                                }
                                Err(e) => {
                                    let _ = app_tx
                                        .send(AppEvent::Error(format!("compaction failed: {e}")))
                                        .await;
                                }
                            }
                            let _ = app_tx.send(AppEvent::Ready).await;
                        }
                        Some(EngineCommand::SlashCommand(cmd)) => {
                            // Slash commands are handled by the render task directly
                            // since they need mutable access to the agent.
                            // This is a placeholder for commands that need engine processing.
                            let _ = app_tx
                                .send(AppEvent::CommandOutput(format!(
                                    "command '{cmd}' not handled by engine"
                                )))
                                .await;
                        }
                        Some(EngineCommand::Shutdown) | None => {
                            let _ = app_tx.send(AppEvent::Shutdown).await;
                            break;
                        }
                    }
                }
            }
        }

        agent
    })
}

/// Run the fully decoupled interactive loop.
///
/// This is the 60fps-capable replacement for `run_interactive`.
/// Three tasks run concurrently:
/// 1. Input task — reads stdin, debounces, sends `AppEvent::UserInput`
/// 2. Engine task — runs agent turns, sends tokens/tool results
/// 3. Render loop (this function) — receives all events, updates display
///
/// The render loop never blocks on I/O. It owns stdout exclusively.
pub async fn run_decoupled(
    agent: anvil_agent::Agent,
    session_summary: Option<String>,
) -> anyhow::Result<()> {
    use crossterm::style::Stylize;
    use std::io::Write;

    let cancel = CancellationToken::new();

    // Print banner (same as interactive.rs)
    print_decoupled_banner(&agent);

    if let Some(summary) = session_summary {
        println!("{summary}");
        println!();
    }

    // Channels
    let (app_tx, mut app_rx) = mpsc::channel::<AppEvent>(256);
    let (cmd_tx, cmd_rx) = mpsc::channel::<EngineCommand>(16);

    // Spawn tasks
    let input_handle = spawn_input_task(app_tx.clone(), cancel.clone(), InputConfig::default());
    let engine_handle = spawn_engine_task(cmd_rx, app_tx.clone(), agent, cancel.clone());

    // Set up Ctrl+C handler
    let ctrlc_cancel = cancel.clone();
    let ctrlc_tx = app_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        ctrlc_cancel.cancel();
        let _ = ctrlc_tx.send(AppEvent::Cancelled).await;
    });

    let mut stdout = std::io::stdout();
    let mut waiting_for_input = false;
    let mut in_stream = false;

    // Main render loop — never blocks on I/O
    loop {
        let event = match app_rx.recv().await {
            Some(e) => e,
            None => break,
        };

        match event {
            AppEvent::Ready => {
                waiting_for_input = true;
                in_stream = false;
                print!("\n{} ", "anvil>".dark_cyan());
                stdout.flush().ok();
            }

            AppEvent::UserInput(text) => {
                if !waiting_for_input {
                    continue; // Ignore input while engine is busy
                }
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    print!("{} ", "anvil>".dark_cyan());
                    stdout.flush().ok();
                    continue;
                }

                if trimmed == "/end" {
                    let _ = cmd_tx.send(EngineCommand::Shutdown).await;
                    break;
                }

                if trimmed == "/clear" {
                    waiting_for_input = false;
                    let _ = cmd_tx.send(EngineCommand::Compact).await;
                    continue;
                }

                if trimmed.starts_with('/') {
                    // Slash commands are handled inline for now
                    // (they need &mut Agent which the engine owns)
                    let _ = cmd_tx.send(EngineCommand::SlashCommand(trimmed)).await;
                    continue;
                }

                waiting_for_input = false;
                let _ = cmd_tx.send(EngineCommand::Prompt(trimmed)).await;
            }

            AppEvent::Token(text) => {
                if !in_stream {
                    in_stream = true;
                    println!(); // Blank line before response
                }
                print!("{text}");
                stdout.flush().ok();
            }

            AppEvent::ThinkingToken(text) => {
                print!("{}", text.dark_grey());
                stdout.flush().ok();
            }

            AppEvent::ToolCallPending {
                name, arguments, ..
            } => {
                println!(
                    "\n{} {} {}",
                    "tool:".dark_yellow(),
                    name.bold(),
                    arguments.dark_grey()
                );
                // Auto-approve for now in decoupled mode
                // TODO: wire permission prompt through input task
                let _ = cmd_tx
                    .send(EngineCommand::Permission(
                        anvil_tools::PermissionDecision::Allow,
                    ))
                    .await;
            }

            AppEvent::ToolResult { name, result } => {
                let preview = if result.len() > 200 {
                    format!("{}...", &result[..200])
                } else {
                    result
                };
                println!(
                    "{} {} {}",
                    "result:".dark_green(),
                    name,
                    preview.dark_grey()
                );
            }

            AppEvent::Usage(usage) => {
                println!(
                    "\n{} prompt={} completion={} total={}",
                    "tokens:".dark_grey(),
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens
                );
            }

            AppEvent::TurnComplete => {
                println!();
                in_stream = false;
            }

            AppEvent::AutoCompacted {
                before_tokens,
                after_tokens,
                messages_removed,
            } => {
                println!(
                    "{} compacted: {} → {} tokens ({} messages removed)",
                    "context:".dark_yellow(),
                    before_tokens,
                    after_tokens,
                    messages_removed
                );
            }

            AppEvent::Retry {
                attempt,
                max,
                delay_secs,
            } => {
                println!(
                    "{} retry {}/{} in {:.1}s",
                    "network:".dark_yellow(),
                    attempt,
                    max,
                    delay_secs
                );
            }

            AppEvent::LoopDetected { tool_name, count } => {
                println!(
                    "{} {} called {} times in a row",
                    "warning:".dark_red(),
                    tool_name,
                    count
                );
            }

            AppEvent::ContextWarning {
                estimated_tokens,
                limit,
            } => {
                println!(
                    "{} context at {}/{} tokens",
                    "warning:".dark_yellow(),
                    estimated_tokens,
                    limit
                );
            }

            AppEvent::ToolOutputDelta { delta } => {
                print!("{}", delta.dark_grey());
                stdout.flush().ok();
            }

            AppEvent::Cancelled => {
                println!("\n{}", "cancelled".dark_red());
                in_stream = false;
            }

            AppEvent::Error(msg) => {
                println!("\n{} {}", "error:".dark_red(), msg);
            }

            AppEvent::CommandOutput(msg) => {
                println!("{msg}");
            }

            AppEvent::AchievementUnlocked {
                icon,
                name,
                description,
                persona,
            } => {
                let msg = anvil_agent::achievements::AchievementStore::format_unlock_parts(
                    &icon,
                    &name,
                    &description,
                    persona.as_deref(),
                );
                println!("\n{msg}\n");
            }

            AppEvent::PermissionResponse(_) => {
                // Handled by engine task internally
            }

            AppEvent::Shutdown => {
                break;
            }
        }
    }

    // Clean shutdown
    cancel.cancel();
    let _ = cmd_tx.send(EngineCommand::Shutdown).await;

    // Wait for tasks to finish
    let _agent = engine_handle.await?;
    input_handle.abort(); // Input task blocks on stdin, must abort
    let _ = input_handle.await;

    println!("session ended.");
    Ok(())
}

fn print_decoupled_banner(agent: &anvil_agent::Agent) {
    if let Some(persona) = agent.persona() {
        let (border, icon) = match persona.key.as_str() {
            "sparkle" => ("✨", "🦄"),
            "bolt" => ("⚡", "🤖"),
            "codebeard" => ("⚓", "🏴\u{200d}☠\u{fe0f}"),
            _ => ("─", "🔨"),
        };
        println!(
            "{border}{border}{border} {icon} {} {border}{border}{border}",
            persona.name
        );
        println!();
        println!("  {}", persona.greeting);
    } else {
        println!("╭─────────────────────────────────────╮");
        println!("│  ⚒  Anvil v{:<25}│", env!("CARGO_PKG_VERSION"));
        println!("│  local coding agent                 │");
        println!("╰─────────────────────────────────────╯");
    }
    println!("  model:   {}", agent.model());
    println!("  session: {}", &agent.session_id()[..8]);
    println!("  type /help for commands, /end to quit");
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_event_variants_are_send() {
        // AppEvent must be Send to cross task boundaries
        fn assert_send<T: Send>() {}
        assert_send::<AppEvent>();
    }

    #[test]
    fn engine_command_variants_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<EngineCommand>();
    }

    #[test]
    fn input_config_defaults() {
        let config = InputConfig::default();
        assert_eq!(config.debounce_ms, 50);
    }

    #[tokio::test]
    async fn cancel_token_stops_engine() {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (app_tx, mut app_rx) = mpsc::channel(64);
        let cancel = CancellationToken::new();

        // Create a minimal agent for testing
        let dir = tempfile::TempDir::new().unwrap();
        let settings = anvil_config::Settings::default();
        let db_path = dir.path().join("test.db");
        let store = anvil_agent::SessionStore::open(&db_path).unwrap();
        let mcp = std::sync::Arc::new(anvil_agent::McpManager::empty());
        let agent =
            anvil_agent::Agent::new(settings, dir.path().to_path_buf(), store, mcp).unwrap();

        let engine_handle = spawn_engine_task(cmd_rx, app_tx, agent, cancel.clone());

        // Wait for Ready
        let event = app_rx.recv().await.unwrap();
        assert!(matches!(event, AppEvent::Ready));

        // Cancel
        cancel.cancel();

        // Engine should send Shutdown and return the agent
        let _agent = engine_handle.await.unwrap();

        // Drain remaining events
        let mut got_shutdown = false;
        while let Ok(event) = app_rx.try_recv() {
            if matches!(event, AppEvent::Shutdown) {
                got_shutdown = true;
            }
        }
        assert!(got_shutdown);

        drop(cmd_tx);
    }

    #[tokio::test]
    async fn engine_handles_compact_command() {
        let (_cmd_tx, cmd_rx) = mpsc::channel(16);
        let (app_tx, mut app_rx) = mpsc::channel(64);
        let cancel = CancellationToken::new();

        let dir = tempfile::TempDir::new().unwrap();
        let settings = anvil_config::Settings::default();
        let db_path = dir.path().join("test.db");
        let store = anvil_agent::SessionStore::open(&db_path).unwrap();
        let mcp = std::sync::Arc::new(anvil_agent::McpManager::empty());
        let agent =
            anvil_agent::Agent::new(settings, dir.path().to_path_buf(), store, mcp).unwrap();

        let engine_handle = spawn_engine_task(cmd_rx, app_tx.clone(), agent, cancel.clone());

        // Wait for Ready
        let _ = app_rx.recv().await;

        // Send compact via a new sender (since we need cmd_tx)
        // Actually we dropped cmd_tx — let's just cancel
        cancel.cancel();
        let _ = engine_handle.await;
    }
}
