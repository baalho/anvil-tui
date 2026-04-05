//! Ralph Loop — autonomous verify-fix cycle.
//!
//! Runs the agent in a loop: prompt → tool execution → verify command →
//! feed failure back → repeat. Extracted from `interactive.rs` since
//! it's a self-contained execution mode.

use anvil_agent::{Agent, AgentEvent, AutonomousConfig, AutonomousRunner, IterationResult};
use anvil_llm::TokenUsage;
use anvil_tools::PermissionDecision;
use anyhow::Result;
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// State needed for Ctrl+C handling during the ralph loop.
pub struct CtrlCState {
    pub pending: AtomicBool,
}

/// Run the autonomous ralph loop.
pub async fn run_ralph_loop(
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
