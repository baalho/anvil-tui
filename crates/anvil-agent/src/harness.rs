//! Long-running multi-agent harness — planner/generator/evaluator.
//!
//! Adapted from the Anthropic harness design for local LLMs on constrained
//! hardware (64GB RAM, 16K–32K context). The key insight: context resets
//! (fresh agent per phase) beat compaction for long-running tasks.
//!
//! # Architecture
//!
//! ```text
//! User prompt → Planner → plan.md
//!                           │
//!                    Sprint Loop:
//!                      Generator → handoff.md
//!                      Evaluator → eval.md (PASS/FAIL)
//!                        fail → retry (max 3)
//!                        pass → next sprint
//!                           │
//!                        DONE
//! ```
//!
//! Each agent is a fresh `Agent` instance. Communication is via structured
//! files in `.anvil/harness/`. No shared context window.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Harness state (state.toml)
// ---------------------------------------------------------------------------

/// Machine-readable harness state persisted to `.anvil/harness/state.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessState {
    pub harness: HarnessMetadata,
    pub usage: HarnessUsage,
}

/// Metadata about the current harness run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessMetadata {
    pub prompt: String,
    pub verify_command: String,
    pub started_at: DateTime<Utc>,
    pub total_sprints: usize,
    pub current_sprint: usize,
    pub current_attempt: usize,
    pub max_attempts_per_sprint: usize,
    pub status: HarnessStatus,
}

/// Token usage across all harness phases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HarnessUsage {
    pub planner_tokens: u64,
    pub generator_tokens: u64,
    pub evaluator_tokens: u64,
    pub total_tokens: u64,
}

/// Overall harness run status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for HarnessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl HarnessState {
    /// Create a new harness state for a fresh run.
    pub fn new(
        prompt: &str,
        verify_command: &str,
        total_sprints: usize,
        max_attempts: usize,
    ) -> Self {
        Self {
            harness: HarnessMetadata {
                prompt: prompt.to_string(),
                verify_command: verify_command.to_string(),
                started_at: Utc::now(),
                total_sprints,
                current_sprint: 0,
                current_attempt: 1,
                max_attempts_per_sprint: max_attempts,
                status: HarnessStatus::Running,
            },
            usage: HarnessUsage::default(),
        }
    }

    /// Load state from `.anvil/harness/state.toml`.
    pub fn load(harness_dir: &Path) -> Result<Self> {
        let path = harness_dir.join("state.toml");
        let content = std::fs::read_to_string(&path)?;
        let state: Self = toml::from_str(&content)?;
        Ok(state)
    }

    /// Save state to `.anvil/harness/state.toml`.
    pub fn save(&self, harness_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(harness_dir)?;
        let content = toml::to_string_pretty(self)?;
        std::fs::write(harness_dir.join("state.toml"), content)?;
        Ok(())
    }

    /// Record token usage from a phase.
    pub fn add_tokens(&mut self, phase: &str, tokens: u64) {
        match phase {
            "planner" => self.usage.planner_tokens += tokens,
            "generator" => self.usage.generator_tokens += tokens,
            "evaluator" => self.usage.evaluator_tokens += tokens,
            _ => {}
        }
        self.usage.total_tokens += tokens;
    }
}

// ---------------------------------------------------------------------------
// Sprint plan (plan.md)
// ---------------------------------------------------------------------------

/// A single sprint parsed from the planner's output.
#[derive(Debug, Clone)]
pub struct SprintPlan {
    /// Sprint index (0-based).
    pub index: usize,
    /// Short title for the sprint.
    pub title: String,
    /// Description of what to implement.
    pub description: String,
    /// Files expected to be involved.
    pub files: Vec<String>,
    /// Acceptance criteria (checkbox items).
    pub criteria: Vec<String>,
    /// Verify command for this sprint (falls back to global verify).
    pub verify_command: Option<String>,
}

impl SprintPlan {
    /// Format this sprint as markdown for injection into the generator's context.
    pub fn as_context(&self, global_verify: &str) -> String {
        let mut out = format!("## Current Sprint: {}\n\n", self.title);
        out.push_str(&self.description);
        out.push('\n');

        if !self.files.is_empty() {
            out.push_str("\n**Files:** ");
            out.push_str(&self.files.join(", "));
            out.push('\n');
        }

        if !self.criteria.is_empty() {
            out.push_str("\n**Acceptance criteria:**\n");
            for c in &self.criteria {
                out.push_str(&format!("- [ ] {c}\n"));
            }
        }

        let verify = self.verify_command.as_deref().unwrap_or(global_verify);
        out.push_str(&format!("\n**Verify:** `{verify}`\n"));

        out
    }
}

/// Parse the planner's markdown output into a list of sprints.
///
/// Tolerant parser: if the output doesn't match the expected format,
/// wraps the entire response as a single sprint. This is the graceful
/// fallback for small models that don't follow format instructions.
pub fn parse_plan(raw: &str, global_verify: &str) -> Vec<SprintPlan> {
    let mut sprints = Vec::new();
    let mut current_title = String::new();
    let mut current_desc = String::new();
    let mut current_files: Vec<String> = Vec::new();
    let mut current_criteria: Vec<String> = Vec::new();
    let mut current_verify: Option<String> = None;
    let mut in_sprint = false;

    for line in raw.lines() {
        let trimmed = line.trim();

        // Detect sprint headers: "## Sprint N: Title" or "## N. Title"
        if let Some(header) = trimmed.strip_prefix("## ") {
            if let Some(title) = parse_sprint_header(header) {
                // Save previous sprint if any
                if in_sprint {
                    sprints.push(SprintPlan {
                        index: sprints.len(),
                        title: current_title.clone(),
                        description: current_desc.trim().to_string(),
                        files: current_files.clone(),
                        criteria: current_criteria.clone(),
                        verify_command: current_verify.clone(),
                    });
                }
                current_title = title;
                current_desc.clear();
                current_files.clear();
                current_criteria.clear();
                current_verify = None;
                in_sprint = true;
                continue;
            }
        }

        if !in_sprint {
            continue;
        }

        // Parse structured fields within a sprint
        if trimmed.starts_with("- **Files:**") || trimmed.starts_with("**Files:**") {
            let files_str = trimmed
                .trim_start_matches("- **Files:**")
                .trim_start_matches("**Files:**")
                .trim();
            current_files = files_str
                .split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect();
        } else if trimmed.starts_with("- **Verify:**") || trimmed.starts_with("**Verify:**") {
            let verify_str = trimmed
                .trim_start_matches("- **Verify:**")
                .trim_start_matches("**Verify:**")
                .trim()
                .trim_matches('`')
                .to_string();
            if !verify_str.is_empty() {
                current_verify = Some(verify_str);
            }
        } else if trimmed.starts_with("- [ ] ") || trimmed.starts_with("- [x] ") {
            let criterion = trimmed[6..].trim().to_string();
            if !criterion.is_empty() {
                current_criteria.push(criterion);
            }
        } else if trimmed.starts_with("- **Description:**")
            || trimmed.starts_with("**Description:**")
        {
            let desc = trimmed
                .trim_start_matches("- **Description:**")
                .trim_start_matches("**Description:**")
                .trim();
            current_desc.push_str(desc);
            current_desc.push('\n');
        } else if !trimmed.starts_with("- **") && !trimmed.starts_with("**") {
            // Regular description text
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                current_desc.push_str(trimmed);
                current_desc.push('\n');
            }
        }
    }

    // Save last sprint
    if in_sprint {
        sprints.push(SprintPlan {
            index: sprints.len(),
            title: current_title,
            description: current_desc.trim().to_string(),
            files: current_files,
            criteria: current_criteria,
            verify_command: current_verify,
        });
    }

    // Fallback: if no sprints were parsed, wrap entire output as single sprint
    if sprints.is_empty() {
        sprints.push(SprintPlan {
            index: 0,
            title: "Complete task".to_string(),
            description: raw.trim().to_string(),
            files: Vec::new(),
            criteria: Vec::new(),
            verify_command: Some(global_verify.to_string()),
        });
    }

    sprints
}

/// Try to extract a sprint title from a header like "Sprint 1: Title" or "1. Title".
fn parse_sprint_header(header: &str) -> Option<String> {
    // "Sprint N: Title"
    if let Some(rest) = header
        .strip_prefix("Sprint ")
        .or_else(|| header.strip_prefix("sprint "))
    {
        if let Some((_num, title)) = rest.split_once(':') {
            return Some(title.trim().to_string());
        }
        // "Sprint N — Title" or just "Sprint N"
        if let Some((_num, title)) = rest.split_once('—') {
            return Some(title.trim().to_string());
        }
        return Some(rest.trim().to_string());
    }

    // "N. Title" or "N: Title"
    let first_char = header.chars().next()?;
    if first_char.is_ascii_digit() {
        if let Some((_num, title)) = header.split_once('.') {
            let t = title.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
        if let Some((_num, title)) = header.split_once(':') {
            let t = title.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }

    None
}

/// Write the plan to `.anvil/harness/plan.md`.
pub fn write_plan(harness_dir: &Path, raw_plan: &str) -> Result<()> {
    std::fs::create_dir_all(harness_dir)?;
    std::fs::write(harness_dir.join("plan.md"), raw_plan)?;
    Ok(())
}

/// Read the plan from `.anvil/harness/plan.md`.
pub fn read_plan(harness_dir: &Path) -> Result<String> {
    let path = harness_dir.join("plan.md");
    let content = std::fs::read_to_string(&path)?;
    Ok(content)
}

// ---------------------------------------------------------------------------
// Handoff document (handoff.md)
// ---------------------------------------------------------------------------

/// Write the generator's handoff to `.anvil/harness/handoff.md`.
pub fn write_handoff(harness_dir: &Path, content: &str) -> Result<()> {
    std::fs::create_dir_all(harness_dir)?;
    // Truncate to ~4000 chars (~1000 tokens) to stay within context budget
    let truncated = if content.len() > 4000 {
        format!("{}...\n[truncated]", &content[..4000])
    } else {
        content.to_string()
    };
    std::fs::write(harness_dir.join("handoff.md"), truncated)?;
    Ok(())
}

/// Read the handoff from `.anvil/harness/handoff.md`.
pub fn read_handoff(harness_dir: &Path) -> Result<String> {
    let path = harness_dir.join("handoff.md");
    if path.exists() {
        Ok(std::fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

// ---------------------------------------------------------------------------
// Evaluation document (eval.md)
// ---------------------------------------------------------------------------

/// Evaluation result from the evaluator agent.
#[derive(Debug, Clone)]
pub struct EvalResult {
    /// Whether the sprint passed evaluation.
    pub passed: bool,
    /// Full evaluation text (written to eval.md).
    pub feedback: String,
}

/// Parse PASS/FAIL from the evaluator's response.
///
/// Looks for "PASS" or "FAIL" (case-insensitive) in the verdict line.
/// Falls back to checking for common patterns. If neither is found,
/// returns None (caller should fall back to verify-command-only).
pub fn parse_eval_verdict(response: &str) -> Option<bool> {
    let upper = response.to_uppercase();

    // Look for explicit verdict markers
    for line in upper.lines() {
        let trimmed = line.trim();
        if trimmed.contains("VERDICT") || trimmed.contains("RESULT") {
            if trimmed.contains("PASS") {
                return Some(true);
            }
            if trimmed.contains("FAIL") {
                return Some(false);
            }
        }
    }

    // Fallback: look for "## Verdict: PASS" or "PASS" / "FAIL" anywhere
    if upper.contains("## VERDICT: PASS") || upper.contains("**PASS**") {
        return Some(true);
    }
    if upper.contains("## VERDICT: FAIL") || upper.contains("**FAIL**") {
        return Some(false);
    }

    // Last resort: count occurrences
    let pass_count = upper.matches("PASS").count();
    let fail_count = upper.matches("FAIL").count();
    if pass_count > 0 && fail_count == 0 {
        return Some(true);
    }
    if fail_count > 0 && pass_count == 0 {
        return Some(false);
    }

    None
}

/// Write the evaluation to `.anvil/harness/eval.md`.
pub fn write_eval(harness_dir: &Path, content: &str) -> Result<()> {
    std::fs::create_dir_all(harness_dir)?;
    std::fs::write(harness_dir.join("eval.md"), content)?;
    Ok(())
}

/// Read the evaluation from `.anvil/harness/eval.md`.
pub fn read_eval(harness_dir: &Path) -> Result<String> {
    let path = harness_dir.join("eval.md");
    if path.exists() {
        Ok(std::fs::read_to_string(&path)?)
    } else {
        Ok(String::new())
    }
}

// ---------------------------------------------------------------------------
// Harness directory
// ---------------------------------------------------------------------------

/// Get the harness directory path for a workspace.
pub fn harness_dir(workspace: &Path) -> PathBuf {
    workspace.join(".anvil").join("harness")
}

/// Check if a harness run is in progress.
pub fn has_active_harness(workspace: &Path) -> bool {
    let dir = harness_dir(workspace);
    let state_path = dir.join("state.toml");
    if !state_path.exists() {
        return false;
    }
    match HarnessState::load(&dir) {
        Ok(state) => state.harness.status == HarnessStatus::Running,
        Err(_) => false,
    }
}

/// Clean up harness artifacts from a previous run.
pub fn clean_harness(workspace: &Path) -> Result<()> {
    let dir = harness_dir(workspace);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Harness events (for UI consumption)
// ---------------------------------------------------------------------------

/// Events emitted by the harness orchestrator for the UI to display.
#[derive(Debug)]
pub enum HarnessEvent {
    /// Planner generated a plan with N sprints.
    PlanGenerated { sprints: usize },
    /// Starting a new sprint.
    SprintStarted {
        index: usize,
        total: usize,
        title: String,
    },
    /// Generator completed a sprint (before evaluation).
    SprintGenerated { index: usize },
    /// Evaluator produced a result.
    SprintEvalResult {
        index: usize,
        passed: bool,
        attempt: usize,
    },
    /// Generator is retrying a failed sprint.
    SprintRetry {
        index: usize,
        attempt: usize,
        max_attempts: usize,
    },
    /// All sprints completed successfully.
    HarnessComplete {
        sprints_completed: usize,
        total_retries: usize,
        elapsed_secs: u64,
    },
    /// Harness stopped due to failure or limits.
    HarnessFailed { sprint: usize, reason: String },
    /// Token usage update from a phase.
    TokenUsage { phase: String, tokens: u64 },
}

// ---------------------------------------------------------------------------
// Done marker for generator sprints
// ---------------------------------------------------------------------------

/// Marker the generator outputs to signal sprint completion.
pub const SPRINT_DONE_MARKER: &str = "[SPRINT:DONE]";

/// Check if a response contains the sprint done marker.
pub fn contains_sprint_done(text: &str) -> bool {
    text.contains(SPRINT_DONE_MARKER)
}

// ---------------------------------------------------------------------------
// System prompts for each agent role
// ---------------------------------------------------------------------------

/// System prompt for the planner agent (~400 tokens).
///
/// The planner decomposes a user's task into ordered sprints. It receives
/// the repo map summary so it knows what files exist, but gets no tool
/// definitions (pure text generation saves ~1K tokens).
pub const PLANNER_PROMPT: &str = r#"You are a project planner. Your job is to decompose a coding task into ordered sprints.

## Output Format

For each sprint, output:

## Sprint N: <short title>
- **Description:** What to implement in this sprint
- **Files:** comma-separated list of files to modify
- **Criteria:**
  - [ ] Specific, testable acceptance criterion
  - [ ] Another criterion
- **Verify:** `command to verify this sprint`

## Rules
- Keep sprints small — each should be completable in 5-15 tool calls
- Order sprints so earlier ones don't depend on later ones
- Be specific about which files to modify
- Write testable acceptance criteria, not vague goals
- If the task is simple enough for one sprint, output one sprint
- Do not write code. Only plan.
"#;

/// System prompt for the evaluator agent (~400 tokens).
///
/// The evaluator is a skeptical critic. It runs the verify command,
/// inspects the code changes, and grades against acceptance criteria.
/// It has limited tools (shell, file_read, grep) — no write access.
pub const EVALUATOR_PROMPT: &str = r#"You are a code evaluator. Your job is to verify whether a sprint was completed correctly.

## Process
1. Read the acceptance criteria for this sprint
2. Read the handoff document to understand what was done
3. Run the verify command using the shell tool
4. Inspect the changed files using file_read and grep
5. Grade each criterion as met or not met

## Output Format

## Verdict: PASS or FAIL

## Criteria
- [x] Criterion that was met
- [ ] Criterion that was NOT met — explain why

## Verify: `<command>`
Exit code: <N>

## Feedback
Specific, actionable feedback for the generator if this is a FAIL.
What exactly needs to be fixed and where.

## Rules
- Be skeptical. Don't assume the work is correct — verify it.
- A single unmet criterion means FAIL.
- If the verify command fails (non-zero exit), that's a FAIL regardless of other criteria.
- Keep feedback specific: file paths, line numbers, exact errors.
- Do NOT modify any files. You are a reviewer, not a developer.
"#;

/// Build the generator's system prompt for a specific sprint.
///
/// Combines the standard Anvil base prompt with sprint-specific context.
/// The sprint context is injected as additional instructions after the
/// base prompt to keep the static prefix stable for KV cache reuse.
pub fn build_generator_prompt(
    sprint: &SprintPlan,
    global_verify: &str,
    handoff: &str,
    eval_feedback: &str,
) -> String {
    let mut prompt = String::with_capacity(4096);

    // Base coding instructions (abbreviated to save tokens)
    prompt.push_str(
        r#"You are a coding assistant. You help with programming tasks by reading, writing, and editing files, and running commands.

## Rules
- Always read a file before editing it.
- Use file_edit for precise changes. Use file_write only for new files.
- Explain briefly what you're doing before taking action.
- When done with all work for this sprint, include [SPRINT:DONE] in your response.

"#,
    );

    // Sprint context
    prompt.push_str(&sprint.as_context(global_verify));

    // Previous sprint handoff
    if !handoff.is_empty() {
        prompt.push_str("\n## Previous Sprint Handoff\n\n");
        prompt.push_str(handoff);
        prompt.push('\n');
    }

    // Evaluator feedback from a failed attempt
    if !eval_feedback.is_empty() {
        prompt.push_str("\n## Previous Evaluation Feedback (FAILED — fix these issues)\n\n");
        prompt.push_str(eval_feedback);
        prompt.push('\n');
    }

    prompt
}

/// Build the evaluator's system prompt with sprint-specific context injected.
pub fn build_evaluator_prompt(sprint: &SprintPlan, global_verify: &str, handoff: &str) -> String {
    let mut prompt = String::with_capacity(2048);
    prompt.push_str(EVALUATOR_PROMPT);

    prompt.push_str("\n---\n\n");
    prompt.push_str(&sprint.as_context(global_verify));

    if !handoff.is_empty() {
        prompt.push_str("\n## Generator Handoff\n\n");
        prompt.push_str(handoff);
        prompt.push('\n');
    }

    prompt
}

/// Build the planner's system prompt with repo map injected.
pub fn build_planner_prompt(repo_map_summary: &str) -> String {
    let mut prompt = String::with_capacity(2048);
    prompt.push_str(PLANNER_PROMPT);

    if !repo_map_summary.is_empty() {
        prompt.push_str("\n---\n\n");
        prompt.push_str(repo_map_summary);
    }

    prompt
}

// ---------------------------------------------------------------------------
// Orchestrator — runs the full planner → generator → evaluator loop
// ---------------------------------------------------------------------------

use crate::agent::AgentEvent;
use crate::Agent;
use anvil_config::Settings;
use anvil_mcp::McpManager;
use anvil_tools::PermissionDecision;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Run the full harness: planner → sprint loop (generator → evaluator) → done.
///
/// Each agent phase creates a fresh `Agent` instance (context reset).
/// Communication between phases is via files in `.anvil/harness/`.
///
/// # Arguments
/// * `settings` — Anvil settings (cloned per agent)
/// * `workspace` — workspace root path
/// * `prompt` — user's task description
/// * `verify_command` — shell command to verify correctness
/// * `event_tx` — channel for UI events
/// * `cancel` — cancellation token (Ctrl+C)
pub async fn run_harness(
    settings: Settings,
    workspace: PathBuf,
    prompt: &str,
    verify_command: &str,
    event_tx: mpsc::Sender<HarnessEvent>,
    cancel: CancellationToken,
) -> Result<()> {
    let start = Instant::now();
    let hdir = harness_dir(&workspace);
    let harness_settings = &settings.harness;

    // Clean up any previous harness run
    clean_harness(&workspace)?;
    std::fs::create_dir_all(&hdir)?;

    // Create a shared MCP manager (empty — harness agents don't use MCP)
    let mcp = Arc::new(McpManager::new(&[]).await);

    // --- Phase 1: Planner ---
    if cancel.is_cancelled() {
        return Ok(());
    }

    let repo_map = crate::repo_map::RepoMap::scan(&workspace);
    let repo_summary = repo_map.summary(4000);
    let planner_prompt = build_planner_prompt(&repo_summary);

    let store = crate::session::SessionStore::open(&hdir.join("planner.db"))?;
    let mut planner = Agent::with_system_prompt(
        settings.clone(),
        workspace.clone(),
        store,
        mcp.clone(),
        &planner_prompt,
        Some(&harness_settings.planner_model),
    )?;

    // Run planner: single turn, no tools
    let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
    let (perm_tx, perm_rx) = mpsc::channel::<PermissionDecision>(1);
    // Auto-approve (planner shouldn't call tools, but just in case)
    tokio::spawn(async move {
        loop {
            if perm_tx.send(PermissionDecision::Allow).await.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    });

    let planner_cancel = cancel.clone();
    planner
        .turn(prompt, &agent_tx, perm_rx, planner_cancel)
        .await?;
    drop(agent_tx);

    // Collect planner output
    let mut plan_text = String::new();
    while let Some(event) = agent_rx.recv().await {
        if let AgentEvent::ContentDelta(delta) = event {
            plan_text.push_str(&delta);
        }
    }

    let planner_tokens = planner.usage().total_tokens;
    drop(planner); // context reset

    if cancel.is_cancelled() {
        return Ok(());
    }

    // Parse plan and write artifacts
    let sprints = parse_plan(&plan_text, verify_command);
    write_plan(&hdir, &plan_text)?;

    let mut state = HarnessState::new(
        prompt,
        verify_command,
        sprints.len(),
        harness_settings.max_retries_per_sprint,
    );
    state.add_tokens("planner", planner_tokens);
    state.save(&hdir)?;

    let _ = event_tx
        .send(HarnessEvent::PlanGenerated {
            sprints: sprints.len(),
        })
        .await;
    let _ = event_tx
        .send(HarnessEvent::TokenUsage {
            phase: "planner".to_string(),
            tokens: planner_tokens,
        })
        .await;

    // --- Phase 2: Sprint loop ---
    let max_sprints = sprints.len().min(harness_settings.max_sprints);
    let mut total_retries = 0;

    for (i, sprint) in sprints.iter().enumerate().take(max_sprints) {
        if cancel.is_cancelled() {
            state.harness.status = HarnessStatus::Cancelled;
            state.save(&hdir)?;
            return Ok(());
        }

        // Check token budget
        if state.usage.total_tokens >= harness_settings.max_total_tokens {
            let _ = event_tx
                .send(HarnessEvent::HarnessFailed {
                    sprint: i,
                    reason: "token budget exceeded".to_string(),
                })
                .await;
            state.harness.status = HarnessStatus::Failed;
            state.save(&hdir)?;
            return Ok(());
        }

        // Check wall-clock timeout
        let elapsed_mins = start.elapsed().as_secs() / 60;
        if elapsed_mins >= harness_settings.max_duration_minutes {
            let _ = event_tx
                .send(HarnessEvent::HarnessFailed {
                    sprint: i,
                    reason: "time limit exceeded".to_string(),
                })
                .await;
            state.harness.status = HarnessStatus::Failed;
            state.save(&hdir)?;
            return Ok(());
        }

        state.harness.current_sprint = i;
        state.harness.current_attempt = 1;
        state.save(&hdir)?;

        let _ = event_tx
            .send(HarnessEvent::SprintStarted {
                index: i,
                total: max_sprints,
                title: sprint.title.clone(),
            })
            .await;

        let mut passed = false;

        for attempt in 1..=harness_settings.max_retries_per_sprint {
            if cancel.is_cancelled() {
                state.harness.status = HarnessStatus::Cancelled;
                state.save(&hdir)?;
                return Ok(());
            }

            state.harness.current_attempt = attempt;
            state.save(&hdir)?;

            // --- Generator ---
            let handoff = read_handoff(&hdir).unwrap_or_default();
            let eval_feedback = if attempt > 1 {
                read_eval(&hdir).unwrap_or_default()
            } else {
                String::new()
            };

            let gen_prompt =
                build_generator_prompt(sprint, verify_command, &handoff, &eval_feedback);

            let gen_store = crate::session::SessionStore::open(&hdir.join("generator.db"))?;
            let mut generator = Agent::with_system_prompt(
                settings.clone(),
                workspace.clone(),
                gen_store,
                mcp.clone(),
                &gen_prompt,
                None, // use default model
            )?;

            // Run generator turns until SPRINT:DONE or turn limit
            let mut gen_content = String::new();
            let turn_limit = harness_settings.sprint_turn_limit;

            for _turn in 0..turn_limit {
                if cancel.is_cancelled() {
                    break;
                }

                let (gtx, mut grx) = mpsc::channel::<AgentEvent>(64);
                let (ptx, prx) = mpsc::channel::<PermissionDecision>(1);

                // Auto-approve all tool calls
                tokio::spawn(async move {
                    loop {
                        if ptx.send(PermissionDecision::Allow).await.is_err() {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                });

                let turn_cancel = cancel.clone();
                let turn_prompt = if gen_content.is_empty() {
                    format!(
                        "Implement the current sprint. When done, include {} in your response.",
                        SPRINT_DONE_MARKER
                    )
                } else {
                    "Continue working on the current sprint. When done, include [SPRINT:DONE] in your response.".to_string()
                };

                generator.turn(&turn_prompt, &gtx, prx, turn_cancel).await?;
                drop(gtx);

                while let Some(event) = grx.recv().await {
                    if let AgentEvent::ContentDelta(delta) = event {
                        gen_content.push_str(&delta);
                    }
                }

                if contains_sprint_done(&gen_content) {
                    break;
                }
            }

            let gen_tokens = generator.usage().total_tokens;
            state.add_tokens("generator", gen_tokens);
            let _ = event_tx
                .send(HarnessEvent::TokenUsage {
                    phase: "generator".to_string(),
                    tokens: gen_tokens,
                })
                .await;

            // Extract handoff from generator output (last ~2000 chars)
            let handoff_content = if gen_content.len() > 2000 {
                &gen_content[gen_content.len() - 2000..]
            } else {
                &gen_content
            };
            write_handoff(&hdir, handoff_content)?;
            drop(generator); // context reset

            let _ = event_tx
                .send(HarnessEvent::SprintGenerated { index: i })
                .await;

            // --- Evaluator ---
            if cancel.is_cancelled() {
                state.harness.status = HarnessStatus::Cancelled;
                state.save(&hdir)?;
                return Ok(());
            }

            let eval_handoff = read_handoff(&hdir).unwrap_or_default();
            let eval_prompt = build_evaluator_prompt(sprint, verify_command, &eval_handoff);

            let eval_store = crate::session::SessionStore::open(&hdir.join("evaluator.db"))?;
            let mut evaluator = Agent::with_system_prompt(
                settings.clone(),
                workspace.clone(),
                eval_store,
                mcp.clone(),
                &eval_prompt,
                Some(&harness_settings.evaluator_model),
            )?;

            // Run evaluator: single turn with tools
            let (etx, mut erx) = mpsc::channel::<AgentEvent>(64);
            let (eptx, eprx) = mpsc::channel::<PermissionDecision>(1);
            tokio::spawn(async move {
                loop {
                    if eptx.send(PermissionDecision::Allow).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            });

            let eval_cancel = cancel.clone();
            let eval_input = format!(
                "Evaluate the sprint. Run the verify command `{}` first, then inspect the changes.",
                verify_command
            );
            evaluator.turn(&eval_input, &etx, eprx, eval_cancel).await?;
            drop(etx);

            let mut eval_content = String::new();
            while let Some(event) = erx.recv().await {
                if let AgentEvent::ContentDelta(delta) = event {
                    eval_content.push_str(&delta);
                }
            }

            let eval_tokens = evaluator.usage().total_tokens;
            state.add_tokens("evaluator", eval_tokens);
            let _ = event_tx
                .send(HarnessEvent::TokenUsage {
                    phase: "evaluator".to_string(),
                    tokens: eval_tokens,
                })
                .await;

            write_eval(&hdir, &eval_content)?;
            drop(evaluator); // context reset

            // Parse verdict
            let verdict = parse_eval_verdict(&eval_content);
            let sprint_passed = match verdict {
                Some(true) => true,
                Some(false) => false,
                None => {
                    // Fallback: run verify command directly
                    let output = std::process::Command::new("sh")
                        .arg("-c")
                        .arg(verify_command)
                        .output();
                    match output {
                        Ok(o) => o.status.success(),
                        Err(_) => false,
                    }
                }
            };

            let _ = event_tx
                .send(HarnessEvent::SprintEvalResult {
                    index: i,
                    passed: sprint_passed,
                    attempt,
                })
                .await;

            state.save(&hdir)?;

            if sprint_passed {
                passed = true;
                break;
            }

            // Retry
            if attempt < harness_settings.max_retries_per_sprint {
                total_retries += 1;
                let _ = event_tx
                    .send(HarnessEvent::SprintRetry {
                        index: i,
                        attempt: attempt + 1,
                        max_attempts: harness_settings.max_retries_per_sprint,
                    })
                    .await;
            }
        }

        if !passed {
            let _ = event_tx
                .send(HarnessEvent::HarnessFailed {
                    sprint: i,
                    reason: format!(
                        "sprint {} failed after {} attempts",
                        i + 1,
                        harness_settings.max_retries_per_sprint
                    ),
                })
                .await;
            state.harness.status = HarnessStatus::Failed;
            state.save(&hdir)?;
            return Ok(());
        }
    }

    // All sprints completed
    state.harness.status = HarnessStatus::Completed;
    state.save(&hdir)?;

    let _ = event_tx
        .send(HarnessEvent::HarnessComplete {
            sprints_completed: max_sprints,
            total_retries,
            elapsed_secs: start.elapsed().as_secs(),
        })
        .await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn state_roundtrip() {
        let dir = TempDir::new().unwrap();
        let state = HarnessState::new("fix all bugs", "cargo test", 5, 3);
        state.save(dir.path()).unwrap();

        let loaded = HarnessState::load(dir.path()).unwrap();
        assert_eq!(loaded.harness.prompt, "fix all bugs");
        assert_eq!(loaded.harness.verify_command, "cargo test");
        assert_eq!(loaded.harness.total_sprints, 5);
        assert_eq!(loaded.harness.current_sprint, 0);
        assert_eq!(loaded.harness.status, HarnessStatus::Running);
        assert_eq!(loaded.usage.total_tokens, 0);
    }

    #[test]
    fn state_add_tokens() {
        let mut state = HarnessState::new("test", "echo ok", 1, 1);
        state.add_tokens("planner", 100);
        state.add_tokens("generator", 500);
        state.add_tokens("evaluator", 200);

        assert_eq!(state.usage.planner_tokens, 100);
        assert_eq!(state.usage.generator_tokens, 500);
        assert_eq!(state.usage.evaluator_tokens, 200);
        assert_eq!(state.usage.total_tokens, 800);
    }

    #[test]
    fn parse_plan_structured() {
        let raw = r#"# Plan: add error handling

## Sprint 1: Handle tools.rs
- **Description:** Add Result returns to all public functions
- **Files:** src/tools.rs, src/executor.rs
- **Criteria:**
  - [ ] All public functions return Result
  - [ ] No unwrap() in public functions
- **Verify:** `cargo test -p anvil-tools`

## Sprint 2: Handle agent.rs
- **Description:** Add error context to agent methods
- **Files:** src/agent.rs
- **Criteria:**
  - [ ] Agent methods use anyhow::Context
- **Verify:** `cargo test -p anvil-agent`
"#;

        let sprints = parse_plan(raw, "cargo test");
        assert_eq!(sprints.len(), 2);

        assert_eq!(sprints[0].title, "Handle tools.rs");
        assert_eq!(sprints[0].files, vec!["src/tools.rs", "src/executor.rs"]);
        assert_eq!(sprints[0].criteria.len(), 2);
        assert_eq!(
            sprints[0].verify_command.as_deref(),
            Some("cargo test -p anvil-tools")
        );

        assert_eq!(sprints[1].title, "Handle agent.rs");
        assert_eq!(sprints[1].index, 1);
    }

    #[test]
    fn parse_plan_numbered_headers() {
        let raw = r#"# Plan

## 1. Fix the parser
Some description here.
- [ ] Parser handles edge cases

## 2. Add tests
Write tests for the parser.
- [ ] Tests cover happy path
"#;

        let sprints = parse_plan(raw, "make test");
        assert_eq!(sprints.len(), 2);
        assert_eq!(sprints[0].title, "Fix the parser");
        assert_eq!(sprints[1].title, "Add tests");
    }

    #[test]
    fn parse_plan_fallback_unstructured() {
        let raw = "Just do the thing. Fix all the bugs and make it work.";
        let sprints = parse_plan(raw, "cargo test");
        assert_eq!(sprints.len(), 1);
        assert_eq!(sprints[0].title, "Complete task");
        assert!(sprints[0].description.contains("Fix all the bugs"));
        assert_eq!(sprints[0].verify_command.as_deref(), Some("cargo test"));
    }

    #[test]
    fn parse_plan_empty_input() {
        let sprints = parse_plan("", "echo ok");
        assert_eq!(sprints.len(), 1);
        assert_eq!(sprints[0].title, "Complete task");
    }

    #[test]
    fn sprint_as_context() {
        let sprint = SprintPlan {
            index: 0,
            title: "Fix parser".to_string(),
            description: "Handle edge cases in the parser".to_string(),
            files: vec!["src/parser.rs".to_string()],
            criteria: vec!["Parser handles empty input".to_string()],
            verify_command: None,
        };

        let ctx = sprint.as_context("cargo test");
        assert!(ctx.contains("Fix parser"));
        assert!(ctx.contains("Handle edge cases"));
        assert!(ctx.contains("src/parser.rs"));
        assert!(ctx.contains("Parser handles empty input"));
        assert!(ctx.contains("`cargo test`"));
    }

    #[test]
    fn sprint_as_context_with_custom_verify() {
        let sprint = SprintPlan {
            index: 0,
            title: "Test".to_string(),
            description: "Test sprint".to_string(),
            files: Vec::new(),
            criteria: Vec::new(),
            verify_command: Some("make test".to_string()),
        };

        let ctx = sprint.as_context("cargo test");
        assert!(ctx.contains("`make test`"));
        assert!(!ctx.contains("cargo test"));
    }

    #[test]
    fn eval_verdict_pass() {
        assert_eq!(
            parse_eval_verdict("## Verdict: PASS\nAll criteria met."),
            Some(true)
        );
        assert_eq!(parse_eval_verdict("**PASS** — looks good"), Some(true));
    }

    #[test]
    fn eval_verdict_fail() {
        assert_eq!(
            parse_eval_verdict("## Verdict: FAIL\nTests don't pass."),
            Some(false)
        );
        assert_eq!(
            parse_eval_verdict("**FAIL** — missing error handling"),
            Some(false)
        );
    }

    #[test]
    fn eval_verdict_ambiguous() {
        // Both PASS and FAIL mentioned — can't determine
        assert_eq!(parse_eval_verdict("Some tests PASS but others FAIL"), None);
    }

    #[test]
    fn eval_verdict_none() {
        assert_eq!(
            parse_eval_verdict("The implementation looks reasonable."),
            None
        );
    }

    #[test]
    fn handoff_truncation() {
        let dir = TempDir::new().unwrap();
        let long_content = "x".repeat(5000);
        write_handoff(dir.path(), &long_content).unwrap();

        let read_back = read_handoff(dir.path()).unwrap();
        assert!(read_back.len() < 4100); // 4000 + "...\n[truncated]"
        assert!(read_back.ends_with("[truncated]"));
    }

    #[test]
    fn handoff_short_content_not_truncated() {
        let dir = TempDir::new().unwrap();
        write_handoff(dir.path(), "short handoff").unwrap();

        let read_back = read_handoff(dir.path()).unwrap();
        assert_eq!(read_back, "short handoff");
    }

    #[test]
    fn has_active_harness_false_when_no_dir() {
        let dir = TempDir::new().unwrap();
        assert!(!has_active_harness(dir.path()));
    }

    #[test]
    fn has_active_harness_true_when_running() {
        let dir = TempDir::new().unwrap();
        let hdir = dir.path().join(".anvil").join("harness");
        let state = HarnessState::new("test", "echo ok", 1, 1);
        state.save(&hdir).unwrap();
        assert!(has_active_harness(dir.path()));
    }

    #[test]
    fn sprint_done_marker() {
        assert!(contains_sprint_done("Done with this sprint. [SPRINT:DONE]"));
        assert!(contains_sprint_done("[SPRINT:DONE]"));
        assert!(!contains_sprint_done("Almost done..."));
        assert!(!contains_sprint_done("[DONE]"));
    }

    #[test]
    fn clean_harness_removes_dir() {
        let dir = TempDir::new().unwrap();
        let hdir = dir.path().join(".anvil").join("harness");
        std::fs::create_dir_all(&hdir).unwrap();
        std::fs::write(hdir.join("state.toml"), "test").unwrap();

        clean_harness(dir.path()).unwrap();
        assert!(!hdir.exists());
    }

    #[test]
    fn plan_file_roundtrip() {
        let dir = TempDir::new().unwrap();
        let plan = "# Plan\n\n## Sprint 1: Do stuff\nDo the stuff.\n";
        write_plan(dir.path(), plan).unwrap();

        let loaded = read_plan(dir.path()).unwrap();
        assert_eq!(loaded, plan);
    }

    // --- Prompt builder tests ---

    #[test]
    fn planner_prompt_includes_repo_map() {
        let prompt = build_planner_prompt("## Repo Map\n\nsrc/main.rs: main\n");
        assert!(prompt.contains("project planner"));
        assert!(prompt.contains("## Repo Map"));
        assert!(prompt.contains("src/main.rs"));
    }

    #[test]
    fn planner_prompt_empty_repo_map() {
        let prompt = build_planner_prompt("");
        assert!(prompt.contains("project planner"));
        assert!(!prompt.contains("---"));
    }

    #[test]
    fn planner_prompt_context_budget() {
        // Planner prompt should be ≤2000 chars (~500 tokens) without repo map
        let prompt = build_planner_prompt("");
        assert!(
            prompt.len() < 2000,
            "planner prompt too large: {} chars",
            prompt.len()
        );
    }

    #[test]
    fn generator_prompt_includes_sprint_context() {
        let sprint = SprintPlan {
            index: 0,
            title: "Fix parser".to_string(),
            description: "Handle edge cases".to_string(),
            files: vec!["src/parser.rs".to_string()],
            criteria: vec!["Parser handles empty input".to_string()],
            verify_command: Some("cargo test".to_string()),
        };

        let prompt = build_generator_prompt(&sprint, "cargo test", "", "");
        assert!(prompt.contains("Fix parser"));
        assert!(prompt.contains("Handle edge cases"));
        assert!(prompt.contains("src/parser.rs"));
        assert!(prompt.contains("[SPRINT:DONE]"));
        assert!(!prompt.contains("Previous Sprint Handoff"));
        assert!(!prompt.contains("Previous Evaluation Feedback"));
    }

    #[test]
    fn generator_prompt_includes_handoff() {
        let sprint = SprintPlan {
            index: 1,
            title: "Step 2".to_string(),
            description: "Continue work".to_string(),
            files: Vec::new(),
            criteria: Vec::new(),
            verify_command: None,
        };

        let prompt = build_generator_prompt(
            &sprint,
            "make test",
            "Sprint 1 completed: added error handling",
            "",
        );
        assert!(prompt.contains("Previous Sprint Handoff"));
        assert!(prompt.contains("added error handling"));
    }

    #[test]
    fn generator_prompt_includes_eval_feedback_on_retry() {
        let sprint = SprintPlan {
            index: 0,
            title: "Fix it".to_string(),
            description: "Fix the bug".to_string(),
            files: Vec::new(),
            criteria: Vec::new(),
            verify_command: None,
        };

        let prompt = build_generator_prompt(
            &sprint,
            "cargo test",
            "",
            "FAIL: tests don't compile, missing import on line 42",
        );
        assert!(prompt.contains("Previous Evaluation Feedback"));
        assert!(prompt.contains("missing import on line 42"));
    }

    #[test]
    fn generator_prompt_context_budget() {
        // Generator prompt without handoff/feedback should be ≤2000 chars (~500 tokens)
        // The sprint context adds more, but base should be small
        let sprint = SprintPlan {
            index: 0,
            title: "Test".to_string(),
            description: "Test sprint".to_string(),
            files: Vec::new(),
            criteria: Vec::new(),
            verify_command: None,
        };
        let prompt = build_generator_prompt(&sprint, "echo ok", "", "");
        assert!(
            prompt.len() < 2500,
            "generator base prompt too large: {} chars",
            prompt.len()
        );
    }

    #[test]
    fn evaluator_prompt_includes_criteria() {
        let sprint = SprintPlan {
            index: 0,
            title: "Add tests".to_string(),
            description: "Write unit tests".to_string(),
            files: vec!["src/lib.rs".to_string()],
            criteria: vec![
                "All functions have tests".to_string(),
                "Coverage above 80%".to_string(),
            ],
            verify_command: Some("cargo test".to_string()),
        };

        let prompt = build_evaluator_prompt(&sprint, "cargo test", "Sprint done");
        assert!(prompt.contains("code evaluator"));
        assert!(prompt.contains("All functions have tests"));
        assert!(prompt.contains("Coverage above 80%"));
        assert!(prompt.contains("Sprint done"));
    }

    #[test]
    fn evaluator_prompt_context_budget() {
        // Evaluator prompt should be ≤2000 chars (~500 tokens) without handoff
        let sprint = SprintPlan {
            index: 0,
            title: "Test".to_string(),
            description: "Test".to_string(),
            files: Vec::new(),
            criteria: Vec::new(),
            verify_command: None,
        };
        let prompt = build_evaluator_prompt(&sprint, "echo ok", "");
        assert!(
            prompt.len() < 2500,
            "evaluator base prompt too large: {} chars",
            prompt.len()
        );
    }

    // --- Edge case tests for plan parser ---

    #[test]
    fn parse_plan_mixed_header_styles() {
        // Models sometimes mix "Sprint N:" and "N." styles
        let raw = r#"# Plan

## Sprint 1: First task
Do the first thing.
- [ ] First criterion

## 2. Second task
Do the second thing.
- [ ] Second criterion
"#;
        let sprints = parse_plan(raw, "echo ok");
        assert_eq!(sprints.len(), 2);
        assert_eq!(sprints[0].title, "First task");
        assert_eq!(sprints[1].title, "Second task");
    }

    #[test]
    fn parse_plan_with_thinking_blocks() {
        // Small models sometimes emit <thinking> blocks before the plan
        let raw = r#"<thinking>
I need to break this into steps...
</thinking>

# Plan

## Sprint 1: Setup
Create the initial structure.
- [ ] Structure exists
"#;
        let sprints = parse_plan(raw, "echo ok");
        assert_eq!(sprints.len(), 1);
        assert_eq!(sprints[0].title, "Setup");
        // Thinking block should not appear in description
        assert!(!sprints[0].description.contains("<thinking>"));
    }

    #[test]
    fn parse_plan_criteria_with_checkmarks() {
        // Models sometimes output [x] instead of [ ]
        let raw = r#"## Sprint 1: Fix bugs
- [x] Already done criterion
- [ ] Not done criterion
"#;
        let sprints = parse_plan(raw, "echo ok");
        assert_eq!(sprints[0].criteria.len(), 2);
        assert_eq!(sprints[0].criteria[0], "Already done criterion");
        assert_eq!(sprints[0].criteria[1], "Not done criterion");
    }

    #[test]
    fn parse_plan_verify_with_backticks() {
        let raw = r#"## Sprint 1: Test
- **Verify:** `cargo test --lib`
"#;
        let sprints = parse_plan(raw, "echo ok");
        assert_eq!(
            sprints[0].verify_command.as_deref(),
            Some("cargo test --lib")
        );
    }

    #[test]
    fn parse_plan_verify_without_backticks() {
        let raw = r#"## Sprint 1: Test
- **Verify:** cargo test --lib
"#;
        let sprints = parse_plan(raw, "echo ok");
        assert_eq!(
            sprints[0].verify_command.as_deref(),
            Some("cargo test --lib")
        );
    }

    #[test]
    fn parse_plan_description_multiline() {
        let raw = r#"## Sprint 1: Complex task
This is a multi-line description.
It spans several lines.
And has details about what to do.
- [ ] Criterion
"#;
        let sprints = parse_plan(raw, "echo ok");
        assert!(sprints[0].description.contains("multi-line description"));
        assert!(sprints[0].description.contains("spans several lines"));
    }

    #[test]
    fn parse_plan_many_sprints() {
        let mut raw = String::from("# Plan\n\n");
        for i in 1..=15 {
            raw.push_str(&format!("## Sprint {i}: Task {i}\nDo task {i}.\n\n"));
        }
        let sprints = parse_plan(&raw, "echo ok");
        assert_eq!(sprints.len(), 15);
        assert_eq!(sprints[14].title, "Task 15");
        assert_eq!(sprints[14].index, 14);
    }

    // --- State machine tests ---

    #[test]
    fn state_status_transitions() {
        let dir = TempDir::new().unwrap();
        let mut state = HarnessState::new("test", "echo ok", 3, 2);
        assert_eq!(state.harness.status, HarnessStatus::Running);

        state.harness.current_sprint = 1;
        state.harness.current_attempt = 2;
        state.save(dir.path()).unwrap();

        let loaded = HarnessState::load(dir.path()).unwrap();
        assert_eq!(loaded.harness.current_sprint, 1);
        assert_eq!(loaded.harness.current_attempt, 2);

        // Transition to failed
        state.harness.status = HarnessStatus::Failed;
        state.save(dir.path()).unwrap();
        let loaded = HarnessState::load(dir.path()).unwrap();
        assert_eq!(loaded.harness.status, HarnessStatus::Failed);

        // Transition to cancelled
        state.harness.status = HarnessStatus::Cancelled;
        state.save(dir.path()).unwrap();
        let loaded = HarnessState::load(dir.path()).unwrap();
        assert_eq!(loaded.harness.status, HarnessStatus::Cancelled);
    }

    #[test]
    fn state_token_accumulation() {
        let mut state = HarnessState::new("test", "echo ok", 1, 1);

        // Simulate multiple generator sprints
        state.add_tokens("planner", 500);
        state.add_tokens("generator", 3000);
        state.add_tokens("evaluator", 1000);
        state.add_tokens("generator", 2500); // second sprint
        state.add_tokens("evaluator", 800);

        assert_eq!(state.usage.planner_tokens, 500);
        assert_eq!(state.usage.generator_tokens, 5500);
        assert_eq!(state.usage.evaluator_tokens, 1800);
        assert_eq!(state.usage.total_tokens, 7800);
    }

    // --- Eval verdict edge cases ---

    #[test]
    fn eval_verdict_case_insensitive() {
        assert_eq!(parse_eval_verdict("## Verdict: pass"), Some(true));
        assert_eq!(parse_eval_verdict("## Verdict: Pass"), Some(true));
        assert_eq!(parse_eval_verdict("## Verdict: Fail"), Some(false));
    }

    #[test]
    fn eval_verdict_in_verbose_response() {
        let response = r#"
I've reviewed the changes carefully.

## Criteria
- [x] All functions return Result
- [x] No unwrap() calls

## Verify: `cargo test`
Exit code: 0

## Verdict: PASS

The implementation looks correct. All acceptance criteria are met.
"#;
        assert_eq!(parse_eval_verdict(response), Some(true));
    }

    #[test]
    fn eval_verdict_fail_with_details() {
        let response = r#"
## Verdict: FAIL

## Criteria
- [x] Functions return Result
- [ ] No unwrap() calls — found unwrap() on line 42 of tools.rs

## Feedback
Fix the unwrap() call on line 42. Use .context("failed to read file")? instead.
"#;
        assert_eq!(parse_eval_verdict(response), Some(false));
    }

    // --- Handoff edge cases ---

    #[test]
    fn handoff_empty_read_returns_empty() {
        let dir = TempDir::new().unwrap();
        // No handoff.md exists
        let content = read_handoff(dir.path()).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn eval_empty_read_returns_empty() {
        let dir = TempDir::new().unwrap();
        let content = read_eval(dir.path()).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn harness_dir_path() {
        let workspace = std::path::Path::new("/home/user/project");
        let hdir = harness_dir(workspace);
        assert_eq!(
            hdir,
            std::path::PathBuf::from("/home/user/project/.anvil/harness")
        );
    }

    #[test]
    fn has_active_harness_false_when_completed() {
        let dir = TempDir::new().unwrap();
        let hdir = dir.path().join(".anvil").join("harness");
        let mut state = HarnessState::new("test", "echo ok", 1, 1);
        state.harness.status = HarnessStatus::Completed;
        state.save(&hdir).unwrap();
        assert!(!has_active_harness(dir.path()));
    }

    #[test]
    fn has_active_harness_false_when_failed() {
        let dir = TempDir::new().unwrap();
        let hdir = dir.path().join(".anvil").join("harness");
        let mut state = HarnessState::new("test", "echo ok", 1, 1);
        state.harness.status = HarnessStatus::Failed;
        state.save(&hdir).unwrap();
        assert!(!has_active_harness(dir.path()));
    }

    #[test]
    fn status_display() {
        assert_eq!(format!("{}", HarnessStatus::Running), "running");
        assert_eq!(format!("{}", HarnessStatus::Completed), "completed");
        assert_eq!(format!("{}", HarnessStatus::Failed), "failed");
        assert_eq!(format!("{}", HarnessStatus::Cancelled), "cancelled");
    }
}
