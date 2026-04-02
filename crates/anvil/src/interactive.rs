use anvil_agent::{
    AchievementStore, Agent, AgentEvent, AutonomousConfig, AutonomousRunner, IterationResult,
    SessionTracker,
};
use anvil_llm::TokenUsage;
use anvil_tools::PermissionDecision;
use anyhow::Result;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::{execute, terminal};
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::commands::{self, CommandResult};
use crate::render::{Renderer, TerminalRenderer};

/// Tracks Ctrl+C state for double-press exit detection.
struct CtrlCState {
    /// Set to true when a Ctrl+C is received during a turn.
    /// If already true when another Ctrl+C arrives, exit the process.
    pending: AtomicBool,
}

pub async fn run_interactive(agent: Agent, session_summary: Option<String>) -> Result<()> {
    // Detect first run — no .anvil/ directory means brand new user
    let is_first_run = !agent.workspace().join(".anvil").exists();

    print_banner(&agent);

    // Show available models hint (async discovery)
    print_model_hint(&agent).await;

    if is_first_run {
        print_first_run_welcome();
    }

    if let Some(summary) = session_summary {
        println!("{summary}");
        println!();
    }

    let renderer = TerminalRenderer::new();
    let stdin = io::stdin();
    let mut cumulative_usage = TokenUsage::default();
    let mut achievement_store = AchievementStore::load(agent.workspace());
    let mut session_tracker = SessionTracker::new();
    let mut last_message_time: Option<std::time::Instant> = None;
    let mut agent_slot: Option<Agent> = Some(agent);
    let mut managed_backend: Option<crate::backend::BackendProcess> = None;
    // Session stats for exit summary
    let session_start = std::time::Instant::now();
    let mut files_created: Vec<String> = Vec::new();
    let mut tool_use_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();

    // Store conversation starters so numeric input (1/2/3) can select them.
    // Populated from the persona's suggestion pool shown in the banner.
    let mut active_suggestions: Vec<String> = {
        let agent_ref = agent_slot.as_ref().expect("agent lost");
        if let Some(persona) = agent_ref.persona() {
            anvil_agent::random_suggestions(persona, 3)
        } else {
            Vec::new()
        }
    };

    // Shared state for Ctrl+C handling
    let ctrlc_state = Arc::new(CtrlCState {
        pending: AtomicBool::new(false),
    });
    // The active cancellation token for the current turn (if any).
    // Wrapped in Arc<Mutex> so the ctrlc handler can access it.
    let active_cancel: Arc<std::sync::Mutex<Option<CancellationToken>>> =
        Arc::new(std::sync::Mutex::new(None));

    // Set up Ctrl+C handler
    {
        let ctrlc_state = ctrlc_state.clone();
        let active_cancel = active_cancel.clone();
        ctrlc::set_handler(move || {
            // If there's an active turn, cancel it
            if let Ok(guard) = active_cancel.lock() {
                if let Some(token) = guard.as_ref() {
                    if !token.is_cancelled() {
                        token.cancel();
                        ctrlc_state.pending.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            }
            // Double Ctrl+C or no active turn — exit immediately
            if ctrlc_state.pending.load(Ordering::SeqCst) {
                // Ensure terminal is in a sane state before exit
                let _ = terminal::disable_raw_mode();
                eprintln!("\nexiting");
                std::process::exit(130);
            }
            // No active turn — just exit
            let _ = terminal::disable_raw_mode();
            eprintln!("\nexiting");
            std::process::exit(130);
        })?;
    }

    loop {
        let agent = agent_slot.as_mut().expect("agent lost");

        // Reset Ctrl+C state between turns
        ctrlc_state.pending.store(false, Ordering::SeqCst);

        // Build status prefix: [persona?|mode|model] icon
        let mode_icon = match agent.mode() {
            anvil_agent::Mode::Coding => "⚒",
            anvil_agent::Mode::Creative => "✨",
        };
        // Override icon if persona is active
        let icon = match agent.persona().map(|p| p.key.as_str()) {
            Some("sparkle") => "🦄",
            Some("bolt") => "🤖",
            Some("codebeard") => "🏴\u{200d}☠\u{fe0f}",
            _ => mode_icon,
        };
        // Shorten model name for display (strip :latest, truncate long names)
        let model_short = agent
            .model()
            .strip_suffix(":latest")
            .unwrap_or(agent.model());
        let status = if let Some(persona) = agent.persona() {
            format!("[{}|{}|{}]", persona.key, agent.mode(), model_short)
        } else {
            format!("[{}|{}]", agent.mode(), model_short)
        };

        // Show token usage in prompt when context is >50% full
        let context_window = agent.context_limit() as u64;
        let used_tokens = cumulative_usage.total_tokens;
        let usage_pct = if context_window > 0 {
            (used_tokens * 100) / context_window
        } else {
            0
        };

        if usage_pct > 80 {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::DarkGrey),
                Print(&status),
                Print(" "),
                SetForegroundColor(Color::Yellow),
                Print(format!(
                    "[{}/{}k ⚠] ",
                    format_token_count(used_tokens),
                    context_window / 1000
                )),
                SetForegroundColor(Color::Green),
                Print(format!("{icon} ▸ ")),
                ResetColor,
            )?;
        } else if usage_pct > 50 {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::DarkGrey),
                Print(&status),
                Print(" "),
                SetForegroundColor(Color::Green),
                Print(format!(
                    "[{}/{}k] ",
                    format_token_count(used_tokens),
                    context_window / 1000
                )),
                Print(format!("{icon} ▸ ")),
                ResetColor,
            )?;
        } else {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::DarkGrey),
                Print(&status),
                Print(" "),
                SetForegroundColor(Color::Green),
                Print(format!("{icon} ▸ ")),
                ResetColor,
            )?;
        }
        io::stdout().flush()?;

        let input = match read_input(&stdin) {
            Some(input) => input,
            None => break,
        };

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Handle numeric input as suggestion selection (1/2/3)
        let trimmed = if !active_suggestions.is_empty() {
            if let Ok(n) = trimmed.parse::<usize>() {
                if n >= 1 && n <= active_suggestions.len() {
                    let suggestion = active_suggestions[n - 1].clone();
                    println!("  → {suggestion}");
                    // Clear suggestions after first use
                    active_suggestions.clear();
                    // Use a leaked string to get a &str with the right lifetime.
                    // This is fine — it happens at most once per session.
                    Box::leak(suggestion.into_boxed_str()) as &str
                } else {
                    trimmed
                }
            } else {
                // Any non-numeric input clears suggestions
                active_suggestions.clear();
                trimmed
            }
        } else {
            trimmed
        };

        // Rate-limit user messages when a kids persona is active.
        // Slash commands bypass the cooldown so /help always works.
        if !trimmed.starts_with('/') {
            if let Some(persona) = agent.persona() {
                if anvil_agent::is_kids_persona(&persona.key) {
                    if let Some(last) = last_message_time {
                        const KIDS_INPUT_COOLDOWN_SECS: u64 = 2;
                        let elapsed = last.elapsed();
                        if elapsed.as_secs() < KIDS_INPUT_COOLDOWN_SECS {
                            let persona_name = persona.name.clone();
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
            }
        }

        if trimmed.starts_with('/') {
            match commands::handle_command(trimmed, agent, &cumulative_usage).await {
                CommandResult::Handled(output) => {
                    if !output.is_empty() {
                        println!("{output}");
                    }
                    continue;
                }
                CommandResult::Compact => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkYellow),
                        Print("  [compacting context...]\n"),
                        ResetColor,
                    )?;
                    io::stdout().flush()?;

                    let cancel = CancellationToken::new();
                    let (event_tx, _event_rx) = mpsc::channel::<AgentEvent>(64);

                    let mut moved_agent = agent_slot.take().unwrap();
                    let compact_handle = tokio::spawn(async move {
                        let result = moved_agent.compact(4, &event_tx, cancel).await;
                        (moved_agent, result)
                    });

                    match compact_handle.await {
                        Ok((returned_agent, Ok(result))) => {
                            agent_slot = Some(returned_agent);
                            if result.messages_removed == 0 {
                                execute!(
                                    io::stdout(),
                                    SetForegroundColor(Color::DarkYellow),
                                    Print("  [nothing to compact]\n"),
                                    ResetColor,
                                )?;
                            } else {
                                execute!(
                                    io::stdout(),
                                    SetForegroundColor(Color::Green),
                                    Print(format!(
                                        "  [compacted: {} messages removed, ~{} → ~{} tokens]\n",
                                        result.messages_removed,
                                        result.before_tokens,
                                        result.after_tokens,
                                    )),
                                    ResetColor,
                                )?;
                            }
                        }
                        Ok((returned_agent, Err(e))) => {
                            agent_slot = Some(returned_agent);
                            execute!(
                                io::stdout(),
                                SetForegroundColor(Color::Red),
                                Print(format!("  [compaction failed: {e}]\n")),
                                ResetColor,
                            )?;
                        }
                        Err(e) => {
                            return Err(anyhow::anyhow!("compaction task panicked: {e}"));
                        }
                    }
                    continue;
                }
                CommandResult::BackendStart(args) => {
                    // Stop existing managed backend if any
                    if let Some(ref mut bp) = managed_backend {
                        bp.stop();
                        managed_backend = None;
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
                                    agent.set_backend(
                                        anvil_config::BackendKind::LlamaServer,
                                        url.clone(),
                                    );
                                    execute!(
                                        io::stdout(),
                                        SetForegroundColor(Color::Green),
                                        Print(format!("backend started: llama-server at {url}\n")),
                                        ResetColor,
                                    )?;
                                    managed_backend = Some(bp);
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
                    continue;
                }
                CommandResult::BackendStop => {
                    if let Some(ref mut bp) = managed_backend {
                        bp.stop();
                        managed_backend = None;
                        execute!(
                            io::stdout(),
                            SetForegroundColor(Color::Green),
                            Print("backend stopped\n"),
                            ResetColor,
                        )?;
                    } else {
                        println!("no managed backend running");
                    }
                    continue;
                }
                CommandResult::Ralph(args) => {
                    let mut moved_agent = agent_slot.take().unwrap();
                    let ctrlc_state_clone = ctrlc_state.clone();
                    let active_cancel_clone = active_cancel.clone();
                    let result = run_ralph_loop(
                        &mut moved_agent,
                        &args,
                        &mut cumulative_usage,
                        &ctrlc_state_clone,
                        &active_cancel_clone,
                    )
                    .await;
                    agent_slot = Some(moved_agent);
                    if let Err(e) = result {
                        execute!(
                            io::stdout(),
                            SetForegroundColor(Color::Red),
                            Print(format!("ralph error: {e}\n")),
                            ResetColor,
                        )?;
                    }
                    continue;
                }
                CommandResult::Exit => break,
                CommandResult::Unknown(cmd) => {
                    eprintln!("unknown command: {cmd} — try /help");
                    continue;
                }
            }
        }

        // Create a cancellation token for this turn
        let cancel = CancellationToken::new();
        {
            let mut guard = active_cancel.lock().unwrap();
            *guard = Some(cancel.clone());
        }

        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
        let (perm_tx, perm_rx) = mpsc::channel::<PermissionDecision>(1);

        // Track user message for achievements and rate limiting
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

        // Spinner with elapsed time while waiting for first LLM token
        let spinner_cancel = CancellationToken::new();
        let spinner_cancel_clone = spinner_cancel.clone();
        let spinner_handle = tokio::spawn(async move {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let start = std::time::Instant::now();
            let mut i = 0;
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(80));
            loop {
                tokio::select! {
                    _ = spinner_cancel_clone.cancelled() => break,
                    _ = interval.tick() => {
                        let frame = FRAMES[i % FRAMES.len()];
                        let elapsed = start.elapsed().as_secs();
                        let timer = if elapsed > 0 {
                            format!(" ({elapsed}s)")
                        } else {
                            String::new()
                        };
                        let _ = execute!(
                            io::stdout(),
                            crossterm::cursor::MoveToColumn(0),
                            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
                            SetForegroundColor(Color::DarkGrey),
                            Print(format!("  {frame} thinking...{timer}")),
                            ResetColor,
                        );
                        let _ = io::stdout().flush();
                        i += 1;
                    }
                }
            }
            // Clear spinner line
            let _ = execute!(
                io::stdout(),
                crossterm::cursor::MoveToColumn(0),
                crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            );
        });

        let mut needs_newline = false;
        let mut spinner_stopped = false;
        let mut spinner_handle = Some(spinner_handle);
        let mut in_thinking_block = false;
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
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::DarkGrey),
                    Print("\n  ╰─\n"),
                    ResetColor,
                )?;
                in_thinking_block = false;
                needs_newline = false;
            }

            match event {
                AgentEvent::ThinkingDelta(text) => {
                    if !in_thinking_block {
                        execute!(
                            io::stdout(),
                            SetForegroundColor(Color::DarkGrey),
                            Print("  ╭─ thinking\n  │ "),
                            ResetColor,
                        )?;
                        in_thinking_block = true;
                        needs_newline = true;
                    }
                    // Prefix each newline with box-drawing continuation
                    let prefixed = text.replace('\n', "\n  │ ");
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(&prefixed),
                        ResetColor,
                    )?;
                    io::stdout().flush()?;
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
                    renderer.render_content_delta(&text);
                }
                AgentEvent::ToolCallPending {
                    name, arguments, ..
                } => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    let icon = tool_icon(&name);
                    let short_args = truncate_display(&arguments, 80);
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Cyan),
                        Print(format!("  {icon} {name}")),
                        SetForegroundColor(Color::DarkGrey),
                        Print(format!(" ─ {short_args}\n")),
                        ResetColor,
                    )?;

                    // Track files created for session summary
                    if name == "file_write" {
                        if let Ok(args) = serde_json::from_str::<serde_json::Value>(&arguments) {
                            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                                files_created.push(path.to_string());
                            }
                        }
                    }

                    // Track tool usage for achievements
                    pending_triggers.extend(session_tracker.record_tool_call(&name, &arguments));

                    let decision = prompt_permission(&name, &arguments)?;
                    let _ = perm_tx.send(decision).await;
                }
                AgentEvent::ToolResult { name, result } => {
                    // Track tool usage for session summary
                    *tool_use_counts.entry(name.clone()).or_insert(0) += 1;

                    let icon = tool_icon(&name);
                    let text = result.text();
                    let lines = text.lines().count();
                    let chars = text.len();
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(format!("  {icon} {name}: {lines} lines, {chars} chars\n")),
                        ResetColor,
                    )?;
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

                    // Unlock any achievements triggered during this turn
                    let agent = agent_slot.as_ref().expect("agent lost");
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
                    // Stream tool output to terminal in real-time
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(&delta),
                        ResetColor,
                    )?;
                }
                AgentEvent::Error(e) => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    renderer.render_error(&e);
                }
            }
        }

        // Clear the active cancel token
        {
            let mut guard = active_cancel.lock().unwrap();
            *guard = None;
        }

        match turn_handle.await {
            Ok((returned_agent, result)) => {
                agent_slot = Some(returned_agent);
                if let Err(e) = result {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Red),
                        Print(format!("error: {e}\n")),
                        ResetColor,
                    )?;
                }
            }
            Err(e) => {
                return Err(anyhow::anyhow!("agent task panicked: {e}"));
            }
        }
    }

    // Stop managed backend if running
    if let Some(ref mut bp) = managed_backend {
        bp.stop();
    }

    // Print session summary
    let is_kids = agent_slot
        .as_ref()
        .and_then(|a| a.persona())
        .map(|p| anvil_agent::is_kids_persona(&p.key))
        .unwrap_or(false);
    print_session_summary(
        session_start.elapsed(),
        &cumulative_usage,
        &tool_use_counts,
        &files_created,
        is_kids,
    );

    if let Some(agent) = agent_slot {
        agent.pause_session()?;
    }
    println!("Session paused. Resume with: anvil --continue");

    Ok(())
}

fn print_banner(agent: &Agent) {
    if let Some(persona) = agent.persona() {
        // Persona-themed banner
        let (border, icon) = match persona.key.as_str() {
            "sparkle" => ("✨", "🦄"),
            "bolt" => ("⚡", "🤖"),
            "codebeard" => ("⚓", "🏴‍☠️"),
            _ => ("─", "🔨"),
        };
        println!(
            "{border}{border}{border} {icon} {} {border}{border}{border}",
            persona.name
        );
        println!();
        println!("  {}", persona.greeting);
        println!();

        // Show conversation starters for kids personas
        let suggestions = anvil_agent::random_suggestions(persona, 3);
        if !suggestions.is_empty() {
            println!("  Try saying:");
            for (i, s) in suggestions.iter().enumerate() {
                println!("    {}. \"{}\"", i + 1, s);
            }
            println!();
        }

        println!("  model:   {}", agent.model());
        println!("  session: {}", &agent.session_id()[..8]);
        println!("  type /help for commands");
        println!();
    } else {
        println!("╭─────────────────────────────────────╮");
        println!("│  ⚒  Anvil v{:<25}│", env!("CARGO_PKG_VERSION"));
        println!("│  local coding agent                 │");
        println!("╰─────────────────────────────────────╯");
        println!("  model:   {}", agent.model());
        println!("  mode:    {}", agent.mode());
        println!("  session: {}", &agent.session_id()[..8]);
        println!("  cwd:     {}", agent.workspace().display());
        // Show last-used profile hint
        if let Some((name, timestamp)) = anvil_config::load_last_profile() {
            let ago = format_time_ago(&timestamp);
            println!("  last profile: {} ({})", name, ago);
            println!("  tip: anvil -p {} to reuse", name);
        }
        println!("  type /help for commands");
        println!();
    }
}

/// Show a hint about available models at startup.
/// Queries the backend and shows count + /model tip if multiple models exist.
async fn print_model_hint(agent: &Agent) {
    use anvil_config::BackendKind;

    let base = agent.base_url().trim_end_matches("/v1");
    let models: Option<Vec<String>> = match agent.backend() {
        BackendKind::Ollama => {
            let url = format!("{base}/api/tags");
            if let Ok(resp) = reqwest::get(&url).await {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    body["models"].as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect()
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }
        BackendKind::LlamaServer | BackendKind::Mlx => {
            let url = format!("{}/models", agent.base_url().trim_end_matches('/'));
            if let Ok(resp) = reqwest::get(&url).await {
                if let Ok(body) = resp.json::<serde_json::Value>().await {
                    body["data"].as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["id"].as_str().map(String::from))
                            .collect()
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }
        BackendKind::Custom => None,
    };

    if let Some(models) = models {
        if models.len() > 1 {
            println!(
                "  models:  {} available — type /model to pick one",
                models.len()
            );
            println!();
        }
    }
}

/// First-run welcome for new users. Detects if this is the first session
/// and offers a friendly introduction with persona selection.
fn print_first_run_welcome() {
    println!("╭─────────────────────────────────────────────────╮");
    println!("│  🎉  Welcome to Anvil!                          │");
    println!("│                                                  │");
    println!("│  Anvil is your coding buddy that runs right      │");
    println!("│  on your computer. No internet needed!           │");
    println!("│                                                  │");
    println!("│  Pick a character to get started:                │");
    println!("│                                                  │");
    println!("│    /persona sparkle   🦄 Sparkle the Unicorn     │");
    println!("│    /persona bolt      🤖 Bolt the Robot          │");
    println!("│    /persona codebeard 🏴‍☠️  Captain Codebeard      │");
    println!("│                                                  │");
    println!("│  Then just say what you like — cats, space,      │");
    println!("│  dragons — and watch something cool happen!      │");
    println!("│                                                  │");
    println!("│  Or just start typing to ask me anything.        │");
    println!("╰─────────────────────────────────────────────────╯");
    println!();
}

fn read_input(stdin: &io::Stdin) -> Option<String> {
    let mut full_input = String::new();
    let reader = stdin.lock();

    for line in reader.lines() {
        match line {
            Ok(line) => {
                if line.ends_with('\\') {
                    full_input.push_str(&line[..line.len() - 1]);
                    full_input.push('\n');
                    // Print continuation prompt
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Green),
                        Print("...  "),
                        ResetColor,
                    )
                    .ok();
                    io::stdout().flush().ok();
                    continue;
                }
                full_input.push_str(&line);
                return Some(full_input);
            }
            Err(_) => return None,
        }
    }

    if full_input.is_empty() {
        None
    } else {
        Some(full_input)
    }
}

fn prompt_permission(tool_name: &str, arguments: &str) -> Result<PermissionDecision> {
    let short_args = truncate_display(arguments, 60);
    execute!(
        io::stdout(),
        SetForegroundColor(Color::Yellow),
        Print(format!("  Allow {tool_name}({short_args})? [y/n/a] ")),
        ResetColor,
    )?;
    io::stdout().flush()?;

    // Read single keypress using crossterm raw mode
    terminal::enable_raw_mode()?;
    let decision = loop {
        if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
            match key.code {
                crossterm::event::KeyCode::Char('y' | 'Y') => {
                    break PermissionDecision::Allow;
                }
                crossterm::event::KeyCode::Char('n' | 'N') => {
                    break PermissionDecision::Deny;
                }
                crossterm::event::KeyCode::Char('a' | 'A') => {
                    break PermissionDecision::AllowAlways;
                }
                crossterm::event::KeyCode::Enter => {
                    break PermissionDecision::Allow;
                }
                _ => {}
            }
        }
    };
    terminal::disable_raw_mode()?;

    let label = match &decision {
        PermissionDecision::Allow => "yes",
        PermissionDecision::Deny => "no",
        PermissionDecision::AllowAlways => "always",
    };
    println!("{label}");

    Ok(decision)
}

/// Print a session summary on exit.
fn print_session_summary(
    duration: std::time::Duration,
    usage: &TokenUsage,
    tool_counts: &std::collections::HashMap<String, u32>,
    files_created: &[String],
    is_kids: bool,
) {
    let mins = duration.as_secs() / 60;
    let secs = duration.as_secs() % 60;
    let duration_str = if mins > 0 {
        format!("{mins} min {secs}s")
    } else {
        format!("{secs}s")
    };

    // Build tool usage string
    let tool_str = if tool_counts.is_empty() {
        "none".to_string()
    } else {
        let mut pairs: Vec<_> = tool_counts.iter().collect();
        pairs.sort_by(|a, b| b.1.cmp(a.1));
        pairs
            .iter()
            .map(|(name, count)| format!("{name} ({count})"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    println!();
    if is_kids {
        println!("╭─ ✨ What You Made! ✨ ──────────────╮");
    } else {
        println!("╭─ Session Summary ─────────────────────╮");
    }
    println!("│  Duration: {:<28}│", duration_str);
    println!(
        "│  Tokens:   {:<28}│",
        format_token_count(usage.total_tokens)
    );
    println!("│  Tools:    {:<28}│", tool_str);

    if !files_created.is_empty() {
        let file_list: Vec<&str> = files_created
            .iter()
            .map(|f| {
                // Show just the filename, not the full path
                f.rsplit('/').next().unwrap_or(f)
            })
            .collect();
        let files_str = if file_list.len() <= 3 {
            file_list.join(", ")
        } else {
            format!(
                "{}, +{} more",
                file_list[..3].join(", "),
                file_list.len() - 3
            )
        };
        println!("│  Files:    {:<28}│", files_str);
    }

    if is_kids {
        let thing_count = files_created.len();
        if thing_count > 0 {
            println!(
                "│                                       │"
            );
            println!(
                "│  ✨ You made {} cool thing{}! ✨        │",
                thing_count,
                if thing_count == 1 { "" } else { "s" }
            );
        }
    }
    println!("╰───────────────────────────────────────╯");
}

/// Format an RFC3339 timestamp as a human-readable "time ago" string.
fn format_time_ago(timestamp: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return "unknown".to_string();
    };
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(then);
    let mins = duration.num_minutes();
    if mins < 1 {
        "just now".to_string()
    } else if mins < 60 {
        format!("{mins} min ago")
    } else if mins < 1440 {
        format!("{} hours ago", mins / 60)
    } else {
        format!("{} days ago", mins / 1440)
    }
}

/// Format token count for compact display (e.g., 1234 → "1.2k", 500 → "500").
fn format_token_count(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

/// Map tool names to display icons for terminal output.
fn tool_icon(name: &str) -> &'static str {
    match name {
        "shell" => "⚙",
        "file_read" => "📄",
        "file_write" => "📝",
        "file_edit" => "✏",
        "grep" => "🔍",
        "find" => "🔍",
        "ls" => "📂",
        "git_status" | "git_diff" | "git_log" | "git_commit" => "📊",
        _ => "🔧", // MCP or plugin tools
    }
}

fn truncate_display(s: &str, max: usize) -> String {
    let oneline = s.replace('\n', " ").replace('\r', "");
    if oneline.len() <= max {
        oneline
    } else {
        format!("{}...", &oneline[..max])
    }
}

/// Run an interactive Ralph Loop inside the interactive session.
///
/// Uses the same AutonomousRunner as CLI mode but integrates with the
/// interactive event loop for display and Ctrl+C cancellation.
async fn run_ralph_loop(
    agent: &mut Agent,
    args: &crate::commands::RalphArgs,
    cumulative_usage: &mut TokenUsage,
    ctrlc_state: &Arc<CtrlCState>,
    active_cancel: &Arc<std::sync::Mutex<Option<CancellationToken>>>,
) -> Result<()> {
    let config = AutonomousConfig {
        verify_command: args.verify_command.clone(),
        max_iterations: args.max_iterations,
        max_tokens: 100_000,
        max_duration: std::time::Duration::from_secs(30 * 60),
    };

    execute!(
        io::stdout(),
        SetForegroundColor(Color::Cyan),
        Print(format!(
            "ralph: max {} iterations, verify: `{}`\n",
            args.max_iterations, args.verify_command
        )),
        ResetColor,
    )?;

    let mut runner = AutonomousRunner::new(config);
    let mut current_prompt = args.prompt.clone();

    loop {
        // Check limits
        let total_tokens = agent.usage().total_tokens;
        if let Some(result) = runner.check_limits(total_tokens) {
            print_ralph_result(&result, &runner)?;
            break;
        }

        runner.next_iteration();
        execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkYellow),
            Print(format!(
                "\n--- iteration {}/{} ({:.0}s elapsed) ---\n",
                runner.iteration(),
                runner.max_iterations(),
                runner.elapsed().as_secs_f64()
            )),
            ResetColor,
        )?;

        // Create cancellation token for this iteration
        let cancel = CancellationToken::new();
        {
            let mut guard = active_cancel.lock().unwrap();
            *guard = Some(cancel.clone());
        }
        ctrlc_state.pending.store(false, Ordering::SeqCst);

        // Run agent turn with auto-approve
        let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
        let (perm_tx, perm_rx) = mpsc::channel::<PermissionDecision>(1);

        // Auto-approve all tool calls in Ralph Loop
        tokio::spawn(async move {
            loop {
                if perm_tx.send(PermissionDecision::Allow).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        // Run the turn directly (not spawned) since we have &mut Agent.
        // The event channel drains after the turn completes because we drop event_tx.
        let turn_cancel = cancel.clone();
        let turn_result = agent
            .turn(&current_prompt, &event_tx, perm_rx, turn_cancel)
            .await;
        drop(event_tx);

        let mut turn_content = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::ContentDelta(text) => {
                    print!("{text}");
                    io::stdout().flush()?;
                    turn_content.push_str(&text);
                }
                AgentEvent::ToolResult { name, result } => {
                    let lines = result.text().lines().count();
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(format!("  [{name}: {lines} lines]\n")),
                        ResetColor,
                    )?;
                }
                AgentEvent::Usage(u) => {
                    cumulative_usage.prompt_tokens += u.prompt_tokens;
                    cumulative_usage.completion_tokens += u.completion_tokens;
                    cumulative_usage.total_tokens += u.total_tokens;
                }
                AgentEvent::Cancelled => {
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkYellow),
                        Print("\n  [ralph loop cancelled]\n"),
                        ResetColor,
                    )?;
                    // Clear active cancel
                    let mut guard = active_cancel.lock().unwrap();
                    *guard = None;
                    return Ok(());
                }
                _ => {}
            }
        }

        if let Err(e) = turn_result {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::Red),
                Print(format!("\nerror in turn: {e}\n")),
                ResetColor,
            )?;
            break;
        }

        // Check for cancellation after turn
        if cancel.is_cancelled() {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::DarkYellow),
                Print("\n  [ralph loop cancelled]\n"),
                ResetColor,
            )?;
            let mut guard = active_cancel.lock().unwrap();
            *guard = None;
            return Ok(());
        }

        // Check for DONE marker
        if anvil_agent::autonomous::contains_done_marker(&turn_content) {
            execute!(
                io::stdout(),
                SetForegroundColor(Color::Cyan),
                Print("\n  [LLM declared DONE — running final verification]\n"),
                ResetColor,
            )?;
        }

        // Run verification
        execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkYellow),
            Print(format!("  [verifying: `{}`]\n", runner.verify_command())),
            ResetColor,
        )?;

        let result = runner.run_verify();
        match &result {
            IterationResult::VerifyPassed { stdout } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Green),
                    Print(format!("  [PASS] {}\n", stdout.trim())),
                    ResetColor,
                )?;
                print_ralph_result(&result, &runner)?;
                break;
            }
            IterationResult::VerifyFailed {
                stdout,
                stderr,
                exit_code,
            } => {
                execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Red),
                    Print(format!("  [FAIL] exit code {exit_code}\n")),
                    ResetColor,
                )?;
                current_prompt = format!(
                    "The verification command `{}` failed (exit code {}).\n\
                     stdout:\n{}\nstderr:\n{}\n\
                     Please fix the issue and try again.",
                    runner.verify_command(),
                    exit_code,
                    stdout.trim(),
                    stderr.trim()
                );
            }
            _ => {
                print_ralph_result(&result, &runner)?;
                break;
            }
        }
    }

    // Clear active cancel
    {
        let mut guard = active_cancel.lock().unwrap();
        *guard = None;
    }

    Ok(())
}

/// Print a summary of the Ralph Loop result.
fn print_ralph_result(result: &IterationResult, runner: &AutonomousRunner) -> Result<()> {
    let elapsed = runner.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;

    let msg = match result {
        IterationResult::VerifyPassed { .. } => {
            format!(
                "ralph: PASSED after {} iterations ({mins}m {secs}s)",
                runner.iteration()
            )
        }
        IterationResult::MaxIterationsReached => {
            format!(
                "ralph: STOPPED — max iterations ({}) reached ({mins}m {secs}s)",
                runner.max_iterations()
            )
        }
        IterationResult::MaxTokensReached => {
            format!("ralph: STOPPED — token budget exceeded ({mins}m {secs}s)")
        }
        IterationResult::TimeoutReached => {
            format!("ralph: STOPPED — time limit reached ({mins}m {secs}s)")
        }
        IterationResult::VerifyFailed { exit_code, .. } => {
            format!("ralph: FAILED — verify exit code {exit_code} ({mins}m {secs}s)")
        }
        IterationResult::LlmDeclaredDone => {
            format!("ralph: LLM declared done ({mins}m {secs}s)")
        }
    };

    execute!(
        io::stdout(),
        SetForegroundColor(Color::Cyan),
        Print(format!("\n{msg}\n")),
        ResetColor,
    )?;

    Ok(())
}
