use anvil_agent::{Agent, AgentEvent, AutonomousConfig, AutonomousRunner, IterationResult};
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

/// Tracks Ctrl+C state for double-press exit detection.
struct CtrlCState {
    /// Set to true when a Ctrl+C is received during a turn.
    /// If already true when another Ctrl+C arrives, exit the process.
    pending: AtomicBool,
}

pub async fn run_interactive(agent: Agent, session_summary: Option<String>) -> Result<()> {
    print_banner(&agent);

    if let Some(summary) = session_summary {
        println!("{summary}");
        println!();
    }

    let stdin = io::stdin();
    let mut cumulative_usage = TokenUsage::default();
    let mut agent_slot: Option<Agent> = Some(agent);
    let mut managed_backend: Option<crate::backend::BackendProcess> = None;

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

        execute!(
            io::stdout(),
            SetForegroundColor(Color::Green),
            Print("you> "),
            ResetColor,
        )?;
        io::stdout().flush()?;

        let input = match read_input(&stdin) {
            Some(input) => input,
            None => break,
        };

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
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

        let prompt_owned = trimmed.to_string();
        let mut moved_agent = agent_slot.take().unwrap();
        let turn_cancel = cancel.clone();
        let turn_handle = tokio::spawn(async move {
            let result = moved_agent
                .turn(&prompt_owned, &event_tx, perm_rx, turn_cancel)
                .await;
            (moved_agent, result)
        });

        let mut needs_newline = false;
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::ThinkingDelta(text) => {
                    if !needs_newline {
                        execute!(
                            io::stdout(),
                            SetForegroundColor(Color::Cyan),
                            Print("anvil> "),
                            ResetColor,
                        )?;
                        needs_newline = true;
                    }
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(&text),
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
                    print!("{text}");
                    io::stdout().flush()?;
                }
                AgentEvent::ToolCallPending {
                    name, arguments, ..
                } => {
                    if needs_newline {
                        println!();
                        needs_newline = false;
                    }
                    let short_args = truncate_display(&arguments, 80);
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Yellow),
                        Print(format!("  [tool: {name}({short_args})]\n")),
                        ResetColor,
                    )?;

                    let decision = prompt_permission(&name, &arguments)?;
                    let _ = perm_tx.send(decision).await;
                }
                AgentEvent::ToolResult { name, result } => {
                    let lines = result.lines().count();
                    let chars = result.len();
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::DarkGrey),
                        Print(format!("  [{name}: {lines} lines, {chars} chars]\n")),
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
                    execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Red),
                        Print(format!("error: {e}\n")),
                        ResetColor,
                    )?;
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

    if let Some(agent) = agent_slot {
        agent.pause_session()?;
    }
    println!("Session paused. Resume with: anvil --continue");

    Ok(())
}

fn print_banner(agent: &Agent) {
    println!("╭─────────────────────────────────────╮");
    println!("│  Anvil — local coding agent         │");
    println!("╰─────────────────────────────────────╯");
    println!("  model:   {}", agent.model());
    println!("  session: {}", &agent.session_id()[..8]);
    println!("  cwd:     {}", agent.workspace().display());
    println!("  type /help for commands");
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
                    let lines = result.lines().count();
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
