//! Autonomous mode (Ralph Loop) — retry-until-verification-passes.
//!
//! # What is the Ralph Loop?
//! Named after the "Ralph Wiggum loop" methodology: continuously feed an AI agent
//! the same task until it completes successfully. The agent runs turns, executes
//! tool calls, then checks a verification command. If verification fails, the
//! failure output is fed back as context for the next iteration.
//!
//! # How it works
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │  1. Send prompt to LLM                      │
//! │  2. Execute tool calls (auto-approved)       │
//! │  3. Run verification command                 │
//! │  4. If verify passes → DONE                  │
//! │  5. If verify fails → feed output back → 1   │
//! │  6. If limits hit → STOP                     │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! # Safety guardrails
//! - **Max iterations**: hard limit on retry count (default: 10)
//! - **Max tokens**: cumulative token budget (default: 100,000)
//! - **Max wall-clock time**: prevents runaway sessions (default: 30 minutes)
//! - **LLM DONE marker**: LLM can declare `[ANVIL:DONE]` to trigger final verify
//!
//! # Verification-based exit
//! The verify command is a shell command (e.g. `cargo test`, `docker compose ps`).
//! Exit code 0 = success. Non-zero = failure. The failure output (stdout+stderr)
//! is fed back to the LLM as a user message for the next iteration.

use std::time::{Duration, Instant};

/// Configuration for autonomous mode.
///
/// All limits are hard stops — the agent will not exceed any of them.
/// The verification command determines success/failure of each iteration.
#[derive(Debug, Clone)]
pub struct AutonomousConfig {
    /// Shell command to run after each iteration. Exit 0 = success.
    pub verify_command: String,
    /// Maximum number of agent turns before giving up.
    pub max_iterations: usize,
    /// Maximum cumulative tokens before stopping.
    pub max_tokens: u64,
    /// Maximum wall-clock time before stopping.
    pub max_duration: Duration,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            verify_command: String::new(),
            max_iterations: 10,
            max_tokens: 100_000,
            max_duration: Duration::from_secs(30 * 60),
        }
    }
}

/// Result of a single autonomous iteration.
#[derive(Debug)]
pub enum IterationResult {
    /// Verification command passed (exit 0).
    VerifyPassed { stdout: String },
    /// Verification command failed — feed output back for next iteration.
    VerifyFailed {
        stdout: String,
        stderr: String,
        exit_code: i32,
    },
    /// LLM declared completion with `[ANVIL:DONE]` marker.
    LlmDeclaredDone,
    /// Hit iteration limit.
    MaxIterationsReached,
    /// Hit token budget.
    MaxTokensReached,
    /// Hit wall-clock time limit.
    TimeoutReached,
}

/// Tracks state across autonomous iterations.
///
/// The runner doesn't own the Agent — it provides the control flow logic
/// while the caller (main.rs / interactive.rs) owns the agent and drives turns.
#[derive(Debug)]
pub struct AutonomousRunner {
    config: AutonomousConfig,
    start_time: Instant,
    iteration: usize,
}

impl AutonomousRunner {
    /// Create a new runner with the given configuration.
    pub fn new(config: AutonomousConfig) -> Self {
        Self {
            config,
            start_time: Instant::now(),
            iteration: 0,
        }
    }

    /// Current iteration number (1-based for display).
    pub fn iteration(&self) -> usize {
        self.iteration
    }

    /// Maximum iterations configured.
    pub fn max_iterations(&self) -> usize {
        self.config.max_iterations
    }

    /// Check if we should continue before starting the next iteration.
    ///
    /// # What gets checked
    /// 1. Iteration count vs max_iterations
    /// 2. Wall-clock time vs max_duration
    /// 3. Token usage vs max_tokens (caller provides current usage)
    ///
    /// Returns `None` if we can continue, or `Some(reason)` if we should stop.
    pub fn check_limits(&self, total_tokens: u64) -> Option<IterationResult> {
        if self.iteration >= self.config.max_iterations {
            return Some(IterationResult::MaxIterationsReached);
        }
        if total_tokens >= self.config.max_tokens {
            return Some(IterationResult::MaxTokensReached);
        }
        if self.start_time.elapsed() >= self.config.max_duration {
            return Some(IterationResult::TimeoutReached);
        }
        None
    }

    /// Increment the iteration counter. Call this before each agent turn.
    pub fn next_iteration(&mut self) {
        self.iteration += 1;
    }

    /// Run the verification command and return the result.
    ///
    /// # How verification works
    /// Runs the verify command via `sh -c` (Unix) or `cmd.exe /C` (Windows).
    /// Captures stdout and stderr. Exit 0 = pass, anything else = fail.
    /// The failure output is returned so the caller can feed it back to the LLM.
    pub fn run_verify(&self) -> IterationResult {
        let output = run_verify_command(&self.config.verify_command);
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                if out.status.success() {
                    IterationResult::VerifyPassed { stdout }
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    IterationResult::VerifyFailed {
                        stdout,
                        stderr,
                        exit_code: out.status.code().unwrap_or(-1),
                    }
                }
            }
            Err(e) => IterationResult::VerifyFailed {
                stdout: String::new(),
                stderr: format!("failed to run verify command: {e}"),
                exit_code: -1,
            },
        }
    }

    /// Elapsed wall-clock time since the autonomous run started.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get the verify command string (for display).
    pub fn verify_command(&self) -> &str {
        &self.config.verify_command
    }
}

/// The marker string the LLM can output to declare task completion.
/// When detected in the LLM's response, triggers one final verification.
pub const DONE_MARKER: &str = "[ANVIL:DONE]";

/// Check if an LLM response contains the done marker.
pub fn contains_done_marker(text: &str) -> bool {
    text.contains(DONE_MARKER)
}

/// Run a shell command for verification.
fn run_verify_command(cmd: &str) -> std::io::Result<std::process::Output> {
    #[cfg(unix)]
    {
        std::process::Command::new("sh").arg("-c").arg(cmd).output()
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd.exe")
            .arg("/C")
            .arg(cmd)
            .output()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = AutonomousConfig::default();
        assert_eq!(config.max_iterations, 10);
        assert_eq!(config.max_tokens, 100_000);
        assert_eq!(config.max_duration, Duration::from_secs(1800));
    }

    #[test]
    fn check_limits_iteration() {
        let config = AutonomousConfig {
            max_iterations: 3,
            ..Default::default()
        };
        let mut runner = AutonomousRunner::new(config);

        assert!(runner.check_limits(0).is_none());
        runner.next_iteration();
        runner.next_iteration();
        runner.next_iteration();
        assert!(matches!(
            runner.check_limits(0),
            Some(IterationResult::MaxIterationsReached)
        ));
    }

    #[test]
    fn check_limits_tokens() {
        let config = AutonomousConfig {
            max_tokens: 1000,
            ..Default::default()
        };
        let runner = AutonomousRunner::new(config);

        assert!(runner.check_limits(999).is_none());
        assert!(matches!(
            runner.check_limits(1000),
            Some(IterationResult::MaxTokensReached)
        ));
    }

    #[test]
    fn done_marker_detection() {
        assert!(contains_done_marker("Task complete. [ANVIL:DONE]"));
        assert!(contains_done_marker("[ANVIL:DONE]"));
        assert!(!contains_done_marker("Almost done..."));
        assert!(!contains_done_marker("[DONE]"));
    }

    #[test]
    fn verify_echo_passes() {
        let config = AutonomousConfig {
            verify_command: "echo ok".to_string(),
            ..Default::default()
        };
        let runner = AutonomousRunner::new(config);
        let result = runner.run_verify();
        assert!(matches!(result, IterationResult::VerifyPassed { .. }));
    }

    #[test]
    fn verify_false_fails() {
        let config = AutonomousConfig {
            verify_command: "false".to_string(),
            ..Default::default()
        };
        let runner = AutonomousRunner::new(config);
        let result = runner.run_verify();
        assert!(matches!(result, IterationResult::VerifyFailed { .. }));
    }

    #[test]
    fn iteration_counter() {
        let config = AutonomousConfig::default();
        let mut runner = AutonomousRunner::new(config);
        assert_eq!(runner.iteration(), 0);
        runner.next_iteration();
        assert_eq!(runner.iteration(), 1);
        runner.next_iteration();
        assert_eq!(runner.iteration(), 2);
    }
}
