//! Interactive readline loop — the primary user interface for Anvil.
//!
//! This module is the orchestrator. It owns the main loop and delegates:
//! - Display (banners, spinners, formatting) → `display.rs`
//! - User input and permission prompts → `prompts.rs`
//! - Autonomous mode (ralph loop) → `ralph.rs`
//! - Slash commands → `commands.rs`
//! - Rendering (tool output, errors) → `render.rs`
//!
//! # TurnPolicy
//! Per-turn behavioral decisions (auto-approve, rate limiting, renderer)
//! are captured in [`TurnPolicy`] rather than scattered `if is_kids`
//! checks. The policy is derived from `Agent::is_kids_mode()` at the
//! start of each loop iteration and threaded through the turn.

use anvil_agent::{AchievementStore, Agent, AgentEvent, SessionTracker};
use anvil_llm::TokenUsage;
use anvil_tools::PermissionDecision;
use anyhow::Result;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::{execute, terminal};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::commands::{self, CommandResult};
use crate::display;
use crate::prompts;
use crate::ralph;
use crate::render::{self, Renderer};

/// Per-turn behavioral policy derived from `Agent::is_kids_mode()`.
///
/// Centralizes the decisions that differ between kids and standard mode:
/// - **auto_approve**: Skip permission prompts (kids can't answer them).
/// - **rate_limit**: Enforce a cooldown between messages (prevent spam).
/// - **renderer**: `KidsRenderer` (fun messages) vs `TerminalRenderer`.
///
/// Built once per loop iteration via [`TurnPolicy::from_agent`], then
/// threaded through the turn without re-querying the agent.
struct TurnPolicy {
    /// Auto-approve all tool calls without prompting.
    auto_approve: bool,
    /// Enforce a cooldown between user messages.
    rate_limit: bool,
    /// Renderer for this turn (kids vs standard).
    renderer: Box<dyn Renderer>,
}

impl TurnPolicy {
    fn from_agent(agent: &Agent) -> Self {
        let kids = agent.is_kids_mode();
        Self {
            auto_approve: kids,
            rate_limit: kids,
            renderer: render::select_renderer(kids),
        }
    }

    fn permission_decision(&self, tool_name: &str, arguments: &str) -> Result<PermissionDecision> {
        if self.auto_approve {
            Ok(PermissionDecision::Allow)
        } else {
            prompts::prompt_permission(tool_name, arguments)
        }
    }
}

/// Tracks Ctrl+C state for double-press exit detection.
struct CtrlCState {
    /// Set to true when a Ctrl+C is received during a turn.
    /// If already true when another Ctrl+C arrives, exit the process.
    pending: AtomicBool,
}

pub async fn run_interactive(agent: Agent, session_summary: Option<String>) -> Result<()> {
    let is_first_run = !agent.workspace().join(".anvil").exists();

    // ── Startup display ──────────────────────────────────────────────
    display::print_banner(&agent);
    display::print_model_hint(&agent).await;
    if is_first_run {
        display::print_first_run_welcome();
    }
    if let Some(summary) = session_summary {
        println!("{summary}");
        println!();
    }

    // ── Session state ────────────────────────────────────────────────
    #[allow(unused_assignments)]
    let mut policy = TurnPolicy::from_agent(&agent);
    let stdin = io::stdin();
    let mut cumulative_usage = TokenUsage::default();
    let mut achievement_store = AchievementStore::load(agent.workspace());
    let mut session_tracker = SessionTracker::new();
    let mut last_message_time: Option<std::time::Instant> = None;
    let mut agent_slot: Option<Agent> = Some(agent);
    let mut managed_backend: Option<crate::backend::BackendProcess> = None;
    let session_start = std::time::Instant::now();
    let mut files_created: Vec<String> = Vec::new();
    let mut tool_use_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    // Conversation starters — numeric input (1/2/3) selects them.
    let mut active_suggestions: Vec<String> = {
        let agent_ref = agent_slot.as_ref().expect("agent lost");
        if let Some(persona) = agent_ref.persona() {
            anvil_agent::random_suggestions(persona, 3)
        } else {
            Vec::new()
        }
    };

    // ── Ctrl+C handling ──────────────────────────────────────────────
    let ctrlc_state = Arc::new(CtrlCState {
        pending: AtomicBool::new(false),
    });
    let active_cancel: Arc<std::sync::Mutex<Option<CancellationToken>>> =
        Arc::new(std::sync::Mutex::new(None));

    {
        let ctrlc_state = ctrlc_state.clone();
        let active_cancel = active_cancel.clone();
        ctrlc::set_handler(move || {
            if let Ok(guard) = active_cancel.lock() {
                if let Some(token) = guard.as_ref() {
                    if !token.is_cancelled() {
                        token.cancel();
                        ctrlc_state.pending.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            }
            if ctrlc_state.pending.load(Ordering::SeqCst) {
                let _ = terminal::disable_raw_mode();
                eprintln!("\nexiting");
                std::process::exit(130);
            }
            let _ = terminal::disable_raw_mode();
            eprintln!("\nexiting");
            std::process::exit(130);
        })?;
    }

    // ── Main loop ────────────────────────────────────────────────────
    loop {
        let agent = agent_slot.as_mut().expect("agent lost");
        ctrlc_state.pending.store(false, Ordering::SeqCst);

        // ── Prompt line ──────────────────────────────────────────────
        policy = TurnPolicy::from_agent(agent);
        let prompt_line = build_prompt_line(agent, &policy, &cumulative_usage);
        execute!(io::stdout(), Print(&prompt_line), ResetColor)?;
        io::stdout().flush()?;

        // ── Read input ───────────────────────────────────────────────
        let input = match prompts::read_input(&stdin) {
            Some(input) => input,
            None => break,
        };

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Handle numeric input as suggestion selection (1/2/3)
        let trimmed = resolve_suggestion(trimmed, &mut active_suggestions);

        // Rate-limit user messages in kids mode
        if !trimmed.starts_with('/') && policy.rate_limit {
            if let Some(last) = last_message_time {
                const KIDS_INPUT_COOLDOWN_SECS: u64 = 2;
                if last.elapsed().as_secs() < KIDS_INPUT_COOLDOWN_SECS {
                    let persona_name = agent
                        .persona()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Anvil".to_string());
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Magenta),
                        Print(format!(
                            "  ✨ {persona_name} is still thinking! Wait a moment...\n"
                        )),
                        ResetColor,
                    )?;
                    continue;
                }
            }
        }

        // ── Slash commands ───────────────────────────────────────────
        if trimmed.starts_with('/') {
            let cmd_result = handle_slash_command(
                trimmed,
                &mut agent_slot,
                &mut policy,
                &mut cumulative_usage,
                &mut managed_backend,
                &ctrlc_state,
                &active_cancel,
            )
            .await?;
            match cmd_result {
                SlashResult::Continue => continue,
                SlashResult::Break => break,
                SlashResult::Panicked(e) => return Err(e),
            }
        }

        // ── Agent turn ───────────────────────────────────────────────
        let cancel = CancellationToken::new();
        {
            let mut guard = active_cancel.lock().unwrap();
            *guard = Some(cancel.clone());
        }

        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
        let (perm_tx, perm_rx) = mpsc::channel::<PermissionDecision>(1);

        last_message_time = Some(std::time::Instant::now());
        let mut pending_triggers: Vec<&'static str> = Vec::new();
        pending_triggers.extend(session_tracker.record_message());

        let prompt_owned = trimmed.to_string();
        let mut moved_agent = agent_slot.take().unwrap();
        let turn_cancel = cancel.clone();
        let turn_handle = tokio::spawn(async move {
            let result = moved_agent
                .turn(&prompt_owned, &event_tx, perm_rx, turn_cancel)
                .await;
            (moved_agent, result)
        });

        // ── Spinner ──────────────────────────────────────────────────
        let spinner_cancel = CancellationToken::new();
        let spinner_handle = display::spawn_spinner(spinner_cancel.clone());

        // ── Event processing ─────────────────────────────────────────
        let mut needs_newline = false;
        let mut spinner_stopped = false;
        let mut spinner_handle = Some(spinner_handle);
        let mut in_thinking_block = false;
        let mut turn_complete_fired = false;

        while let Some(event) = event_rx.recv().await {
            // Stop spinner on first event
            if !spinner_stopped {
                spinner_cancel.cancel();
                if let Some(handle) = spinner_handle.take() {
                    let _ = handle.await;
                }
                spinner_stopped = true;
            }

            // Close thinking box when transitioning to non-thinking events
            if in_thinking_block && !matches!(event, AgentEvent::ThinkingDelta(_)) {
                policy.renderer.render_thinking_end();
                in_thinking_block = false;
                needs_newline = false;
            }

            match event {
                AgentEvent::ThinkingDelta(text) => {
                    if !in_thinking_block {
                        policy.renderer.render_thinking_start();
                        in_thinking_block = true;
                        needs_newline = true;
                    }
                    policy.renderer.render_thinking_delta(&text);
                }
                AgentEvent::ContentDelta(text) => {
                    if !needs_newline {
                        execute!(
                            io::stdout(),
                            SetForegroundColor(Color::Cyan),
                            Print("anvil> "),
                            ResetColor,
                        )?;
                        needs_newline = true;
                    }
                    policy.renderer.render_content_delta(&text);
                }
                AgentEvent::ToolCallPending {
                    name, arguments, ..
                } => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }

                    let icon = display::tool_icon(&name);
                    let short_args = display::truncate_display(&arguments, 80);
                    policy
                        .renderer
                        .render_tool_pending(&name, icon, &short_args);

                    if name == "file_write" {
                        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&arguments) {
                            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                                files_created.push(path.to_string());
                            }
                        }
                    }

                    pending_triggers.extend(session_tracker.record_tool_call(&name, &arguments));

                    let decision = policy.permission_decision(&name, &arguments)?;
                    let _ = perm_tx.send(decision).await;
                }
                AgentEvent::ToolResult { name, result } => {
                    *tool_use_counts.entry(name.clone()).or_insert(0) += 1;
                    let icon = display::tool_icon(&name);
                    policy.renderer.render_tool_output(&name, icon, &result);
                }
                AgentEvent::Usage(u) => {
                    cumulative_usage.prompt_tokens += u.prompt_tokens;
                    cumulative_usage.completion_tokens += u.completion_tokens;
                    cumulative_usage.total_tokens += u.total_tokens;
                }
                AgentEvent::TurnComplete => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    turn_complete_fired = true;
                    println!();
                }
                AgentEvent::AutoCompacted {
                    before_tokens,
                    after_tokens,
                    messages_removed,
                } => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkYellow),
                        Print(format!(
                            "  [auto-compacted: {messages_removed} messages, ~{before_tokens} → ~{after_tokens} tokens]\n"
                        )),
                        ResetColor,
                    )?;
                }
                AgentEvent::Cancelled => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkYellow),
                        Print("  [cancelled]\n"),
                        ResetColor,
                    )?;
                    println!();
                }
                AgentEvent::Retry {
                    attempt,
                    max,
                    delay_secs,
                } => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkYellow),
                        Print(format!(
                            "  [retrying in {delay_secs:.1}s... (attempt {attempt}/{max})]\n"
                        )),
                        ResetColor,
                    )?;
                }
                AgentEvent::LoopDetected { tool_name, count } => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Red),
                        Print(format!("  [loop detected: {tool_name} x{count}]\n")),
                        ResetColor,
                    )?;
                }
                AgentEvent::ContextWarning {
                    estimated_tokens,
                    limit,
                } => {
                    let pct = (estimated_tokens * 100) / limit;
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkYellow),
                        Print(format!(
                            "  [context: ~{estimated_tokens}/{limit} tokens ({pct}%)]\n"
                        )),
                        ResetColor,
                    )?;
                }
                AgentEvent::ToolOutputDelta { delta, .. } => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(&delta),
                        ResetColor,
                    )?;
                }
                AgentEvent::ModelSwitched { from, to } => {
                    policy
                        .renderer
                        .render_info(&format!("  [routing: {} → {}]", from, to));
                }
                AgentEvent::Error(e) => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    policy.renderer.render_error(&e);
                }
            }
        }

        // ── Post-turn ────────────────────────────────────────────────
        {
            let mut guard = active_cancel.lock().unwrap();
            *guard = None;
        }

        match turn_handle.await {
            Ok((returned_agent, result)) => {
                agent_slot = Some(returned_agent);

                if turn_complete_fired {
                    if let Some(agent) = agent_slot.as_ref() {
                        let persona = agent.persona().map(|p| p.key.as_str());
                        for key in pending_triggers.drain(..) {
                            if let Some(badge) = achievement_store.unlock(key, persona) {
                                let msg = AchievementStore::format_unlock(badge, persona);
                                execute!(
                                    io::stdout(),
                                    SetForegroundColor(Color::Yellow),
                                    Print(format!("  {msg}\n")),
                                    ResetColor,
                                )?;
                            }
                        }
                    }
                }

                if let Err(e) = result {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Red),
                        Print(format!("error: {e}\n")),
                        ResetColor,
                    )?;
                }

                // Warn about uncommitted changes after file-modifying turns
                if !files_created.is_empty() {
                    if let Some(agent) = agent_slot.as_ref() {
                        if let Ok(status) = crate::commands::check_uncommitted(agent.workspace()) {
                            if status.changed_files >= 10 {
                                execute!(
                                    io::stdout(),
                                    SetForegroundColor(Color::DarkYellow),
                                    Print(format!(
                                        "  [⚠ {} uncommitted changes across {} files — consider /commit]\n",
                                        status.changed_files, status.unique_files
                                    )),
                                    ResetColor,
                                )?;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("agent task panicked: {e}"));
            }
        }
    }

    // ── Shutdown ─────────────────────────────────────────────────────
    if let Some(ref mut bp) = managed_backend {
        bp.stop();
    }

    let final_policy = agent_slot
        .as_ref()
        .map(TurnPolicy::from_agent)
        .unwrap_or_else(|| TurnPolicy {
            auto_approve: false,
            rate_limit: false,
            renderer: render::select_renderer(false),
        });
    display::print_session_summary(
        session_start.elapsed(),
        &cumulative_usage,
        &tool_use_counts,
        &files_created,
        final_policy.renderer.as_ref(),
    );

    if let Some(agent) = agent_slot {
        agent.pause_session()?;
    }
    println!("Session paused. Resume with: anvil --continue");

    Ok(())
}

// ── Helper functions ─────────────────────────────────────────────────

/// Build the prompt line with mode icon, model name, and token usage.
fn build_prompt_line(agent: &Agent, policy: &TurnPolicy, usage: &TokenUsage) -> String {
    let mode_icon = match agent.mode() {
        anvil_agent::Mode::Coding => "⚒",
        anvil_agent::Mode::Creative => "✨",
    };
    let icon = match agent.persona().map(|p| p.key.as_str()) {
        Some("sparkle") => "🦄",
        Some("bolt") => "🤖",
        Some("codebeard") => "🏴\u{200d}☠\u{fe0f}",
        _ => mode_icon,
    };
    let model_short = agent
        .model()
        .strip_suffix(":latest")
        .unwrap_or(agent.model());
    let persona = agent.persona();

    let status = policy.renderer.format_status(
        &agent.mode().to_string(),
        model_short,
        persona.as_ref().map(|p| p.key.as_str()),
    );

    let context_window = agent.context_limit() as u64;
    let used_tokens = usage.total_tokens;
    let usage_pct = if context_window > 0 {
        (used_tokens * 100) / context_window
    } else {
        0
    };

    if usage_pct > 80 {
        format!(
            "\x1b[90m{status}\x1b[0m \x1b[33m[{}/{}k ⚠]\x1b[0m \x1b[32m{icon} ▸ \x1b[0m",
            display::format_token_count(used_tokens),
            context_window / 1000
        )
    } else if usage_pct > 50 {
        format!(
            "\x1b[90m{status}\x1b[0m \x1b[32m[{}/{}k]\x1b[0m \x1b[32m{icon} ▸ \x1b[0m",
            display::format_token_count(used_tokens),
            context_window / 1000
        )
    } else {
        format!("\x1b[90m{status}\x1b[0m \x1b[32m{icon} ▸ \x1b[0m")
    }
}

/// Resolve numeric input (1/2/3) to a suggestion, or return the original input.
fn resolve_suggestion<'a>(trimmed: &'a str, suggestions: &mut Vec<String>) -> &'a str {
    if suggestions.is_empty() {
        return trimmed;
    }
    if let Ok(n) = trimmed.parse::<usize>() {
        if n >= 1 && n <= suggestions.len() {
            let suggestion = suggestions[n - 1].clone();
            println!("  → {suggestion}");
            suggestions.clear();
            // Leak is fine — happens at most once per session
            return Box::leak(suggestion.into_boxed_str());
        }
    }
    // Any non-numeric input clears suggestions
    suggestions.clear();
    trimmed
}

/// Result of handling a slash command.
enum SlashResult {
    Continue,
    Break,
    Panicked(anyhow::Error),
}

/// Handle all slash command variants, returning what the main loop should do.
/// Run the multi-agent harness from a `/harness` command.
async fn run_harness_command(
    args: &commands::HarnessArgs,
    agent_slot: &mut Option<Agent>,
    active_cancel: &Arc<std::sync::Mutex<Option<CancellationToken>>>,
) -> Result<()> {
    use anvil_agent::harness::{self, HarnessEvent};

    let agent = agent_slot.as_ref().unwrap();
    let settings = agent.settings().clone();
    let workspace = agent.workspace().to_path_buf();

    // Create a cancellation token and register it so Ctrl+C can cancel
    let cancel = CancellationToken::new();
    {
        let mut guard = active_cancel.lock().unwrap();
        *guard = Some(cancel.clone());
    }

    let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(64);
    let harness_cancel = cancel.clone();

    let prompt = args.prompt.clone();
    let verify = args.verify_command.clone();

    // Print header
    execute!(
        io::stdout(),
        SetForegroundColor(Color::Cyan),
        Print("⚙ harness: "),
        ResetColor,
        Print(format!("{}\n", prompt)),
        SetForegroundColor(Color::DarkGrey),
        Print(format!("  verify: {verify}\n\n")),
        ResetColor,
    )?;

    // Spawn the harness orchestrator
    let harness_handle = tokio::spawn(async move {
        harness::run_harness(
            settings,
            workspace,
            &prompt,
            &verify,
            event_tx,
            harness_cancel,
        )
        .await
    });

    // Process events from the harness
    while let Some(event) = event_rx.recv().await {
        match event {
            HarnessEvent::PlanGenerated { sprints } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Green),
                    Print(format!("  ✓ plan: {sprints} sprints\n")),
                    ResetColor,
                )?;
            }
            HarnessEvent::SprintStarted {
                index,
                total,
                title,
            } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Cyan),
                    Print(format!("\n  [{}/{}] {title}\n", index + 1, total)),
                    ResetColor,
                )?;
            }
            HarnessEvent::SprintGenerated { index: _ } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::DarkGrey),
                    Print("    generator done, evaluating...\n"),
                    ResetColor,
                )?;
            }
            HarnessEvent::SprintEvalResult {
                index: _,
                passed,
                attempt,
            } => {
                if passed {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Green),
                        Print(format!("    ✓ PASS (attempt {attempt})\n")),
                        ResetColor,
                    )?;
                } else {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Red),
                        Print(format!("    ✗ FAIL (attempt {attempt})\n")),
                        ResetColor,
                    )?;
                }
            }
            HarnessEvent::SprintRetry {
                index: _,
                attempt,
                max_attempts,
            } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Yellow),
                    Print(format!("    retrying ({attempt}/{max_attempts})...\n")),
                    ResetColor,
                )?;
            }
            HarnessEvent::HarnessComplete {
                sprints_completed,
                total_retries,
                elapsed_secs,
            } => {
                let mins = elapsed_secs / 60;
                let secs = elapsed_secs % 60;
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Green),
                    Print(format!(
                        "\n  ✓ harness complete: {sprints_completed} sprints, \
                         {total_retries} retries, {mins}m{secs}s\n"
                    )),
                    ResetColor,
                )?;
            }
            HarnessEvent::HarnessFailed { sprint, reason } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Red),
                    Print(format!(
                        "\n  ✗ harness failed at sprint {}: {reason}\n",
                        sprint + 1
                    )),
                    ResetColor,
                )?;
            }
            HarnessEvent::TokenUsage { phase, tokens } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("    tokens ({phase}): {tokens}\n")),
                    ResetColor,
                )?;
            }
        }
    }

    // Wait for the harness to finish
    match harness_handle.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::Red),
                Print(format!("\n  harness error: {e}\n")),
                ResetColor,
            )?;
        }
        Err(e) => {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::Red),
                Print(format!("\n  harness task error: {e}\n")),
                ResetColor,
            )?;
        }
    }

    // Clear the cancel token
    {
        let mut guard = active_cancel.lock().unwrap();
        *guard = None;
    }

    Ok(())
}

async fn handle_slash_command(
    trimmed: &str,
    agent_slot: &mut Option<Agent>,
    policy: &mut TurnPolicy,
    cumulative_usage: &mut TokenUsage,
    managed_backend: &mut Option<crate::backend::BackendProcess>,
    _ctrlc_state: &Arc<CtrlCState>,
    active_cancel: &Arc<std::sync::Mutex<Option<CancellationToken>>>,
) -> Result<SlashResult> {
    let agent = agent_slot.as_mut().expect("agent lost");

    match commands::handle_command(trimmed, agent, cumulative_usage).await {
        CommandResult::Handled(output) => {
            if !output.is_empty() {
                policy.renderer.render_command_result(&output);
            }
            Ok(SlashResult::Continue)
        }
        CommandResult::Compact => {
            policy.renderer.render_info("  [compacting context...]");

            let cancel = CancellationToken::new();
            let (event_tx, _event_rx) = mpsc::channel::<AgentEvent>(64);

            let mut moved_agent = agent_slot.take().unwrap();
            let compact_handle = tokio::spawn(async move {
                let result = moved_agent.compact(4, &event_tx, cancel).await;
                (moved_agent, result)
            });

            match compact_handle.await {
                Ok((returned_agent, Ok(result))) => {
                    *agent_slot = Some(returned_agent);
                    if result.messages_removed == 0 {
                        policy.renderer.render_info("  [nothing to compact]");
                    } else {
                        policy.renderer.render_info(&format!(
                            "  [compacted: {} messages removed, ~{} → ~{} tokens]",
                            result.messages_removed, result.before_tokens, result.after_tokens,
                        ));
                    }
                }
                Ok((returned_agent, Err(e))) => {
                    *agent_slot = Some(returned_agent);
                    policy
                        .renderer
                        .render_error(&format!("compaction failed: {e}"));
                }
                Err(e) => {
                    return Ok(SlashResult::Panicked(anyhow::anyhow!(
                        "compaction task panicked: {e}"
                    )));
                }
            }
            Ok(SlashResult::Continue)
        }
        CommandResult::BackendStart(args) => {
            if let Some(ref mut bp) = managed_backend {
                bp.stop();
                *managed_backend = None;
            }

            match args.backend_type.as_str() {
                "llama" | "llama-server" => {
                    match crate::backend::BackendProcess::start_llama_server(
                        &args.model_path,
                        args.port,
                        &[],
                    )
                    .await
                    {
                        Ok(bp) => {
                            let url = bp.base_url().to_string();
                            let agent = agent_slot.as_mut().expect("agent lost");
                            agent.set_backend(anvil_config::BackendKind::LlamaServer, url.clone());
                            execute!(
                                io::stdout(),
                                SetForegroundColor(Color::Green),
                                Print(format!("backend started: llama-server at {url}\n")),
                                ResetColor,
                            )?;
                            *managed_backend = Some(bp);
                        }
                        Err(e) => {
                            execute!(
                                io::stdout(),
                                SetForegroundColor(Color::Red),
                                Print(format!("failed to start backend: {e}\n")),
                                ResetColor,
                            )?;
                        }
                    }
                }
                other => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Red),
                        Print(format!(
                            "managed start not supported for '{other}'. Use: llama\n"
                        )),
                        ResetColor,
                    )?;
                }
            }
            Ok(SlashResult::Continue)
        }
        CommandResult::BackendStop => {
            if let Some(ref mut bp) = managed_backend {
                bp.stop();
                *managed_backend = None;
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Green),
                    Print("backend stopped\n"),
                    ResetColor,
                )?;
            } else {
                println!("no managed backend running");
            }
            Ok(SlashResult::Continue)
        }
        CommandResult::Ralph(args) => {
            let mut moved_agent = agent_slot.take().unwrap();
            let result = ralph::run_ralph_loop(
                &mut moved_agent,
                &args,
                cumulative_usage,
                &Arc::new(ralph::CtrlCState {
                    pending: AtomicBool::new(false),
                }),
                active_cancel,
            )
            .await;
            *agent_slot = Some(moved_agent);
            if let Err(e) = result {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Red),
                    Print(format!("ralph error: {e}\n")),
                    ResetColor,
                )?;
            }
            Ok(SlashResult::Continue)
        }
        CommandResult::Harness(args) => {
            run_harness_command(&args, agent_slot, active_cancel).await?;
            Ok(SlashResult::Continue)
        }
        CommandResult::Exit => Ok(SlashResult::Break),
        CommandResult::Unknown(cmd) => {
            eprintln!("unknown command: {cmd} — try /help");
            Ok(SlashResult::Continue)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_suggestion_numeric() {
        let mut suggestions = vec![
            "Make something sparkly".to_string(),
            "Tell me a joke".to_string(),
        ];
        let result = resolve_suggestion("1", &mut suggestions);
        assert_eq!(result, "Make something sparkly");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn resolve_suggestion_non_numeric_clears() {
        let mut suggestions = vec!["hello".to_string()];
        let result = resolve_suggestion("hello world", &mut suggestions);
        assert_eq!(result, "hello world");
        assert!(suggestions.is_empty());
    }

    #[test]
    fn resolve_suggestion_out_of_range() {
        let mut suggestions = vec!["hello".to_string()];
        let result = resolve_suggestion("5", &mut suggestions);
        assert_eq!(result, "5");
        // Out-of-range numeric input clears suggestions (same as non-numeric)
        assert!(suggestions.is_empty());
    }
}
