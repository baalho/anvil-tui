pub mod app;
mod backend;
mod commands;
mod interactive;

use anvil_agent::{Agent, AgentEvent, McpManager, McpServerConfig, SessionStore};
use anvil_config::{data_dir, load_settings, Settings};
use anvil_tools::PermissionDecision;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    name = "anvil",
    version,
    about = "A local-first coding agent for local models."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Working directory (defaults to current directory)
    #[arg(short = 'C', long)]
    directory: Option<PathBuf>,

    /// Resume the most recent session, or a specific session by ID prefix
    #[arg(short = 'c', long = "continue")]
    continue_session: Option<Option<String>>,

    /// Use the decoupled async TUI (60fps, non-blocking)
    #[arg(long = "tui")]
    decoupled_tui: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new .anvil/ harness directory
    Init,
    /// List past sessions
    History {
        #[arg(short, long, default_value = "20")]
        limit: usize,
        /// Search session content
        #[arg(short, long)]
        search: Option<String>,
    },
    /// Show detailed documentation on a topic (tools, skills, config, commands)
    Docs {
        /// Topic to show docs for
        topic: String,
    },
    /// Run a single prompt non-interactively
    Run {
        /// The prompt to send
        #[arg(short, long)]
        prompt: String,
        /// Auto-approve all tool calls
        #[arg(short = 'y', long = "yes")]
        auto_approve: bool,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        output: String,
        /// Run in autonomous mode (Ralph Loop) — retry until verification passes
        #[arg(short = 'a', long)]
        autonomous: bool,
        /// Shell command to verify success (exit 0 = pass). Required for --autonomous
        #[arg(long, requires = "autonomous")]
        verify: Option<String>,
        /// Maximum iterations in autonomous mode
        #[arg(long, default_value = "10")]
        max_iterations: usize,
        /// Maximum wall-clock minutes in autonomous mode
        #[arg(long, default_value = "30")]
        max_minutes: u64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("anvil=info".parse()?),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let workspace = cli
        .directory
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match cli.command {
        Some(Commands::Init) => cmd_init(&workspace),
        Some(Commands::Docs { topic }) => cmd_help(&topic),
        Some(Commands::History { limit, search }) => cmd_history(limit, search),
        Some(Commands::Run {
            prompt,
            auto_approve,
            output,
            autonomous,
            verify,
            max_iterations,
            max_minutes,
        }) => {
            if autonomous {
                let verify_cmd = verify.unwrap_or_else(|| "echo ok".to_string());
                cmd_run_autonomous(
                    &workspace,
                    &prompt,
                    &verify_cmd,
                    max_iterations,
                    max_minutes,
                )
                .await
            } else {
                cmd_run(&workspace, &prompt, auto_approve, &output).await
            }
        }
        None => {
            let mut settings = load_settings(&workspace)?;
            auto_detect_model(&mut settings).await;

            // Load model profiles and apply matching profile's sampling params
            let profiles = load_model_profiles(&workspace);
            let active_profile =
                anvil_config::find_matching_profile(&profiles, &settings.provider.model);
            if let Some(profile) = active_profile {
                // Override context window from profile if it provides one
                if profile.context.default_window > 0 {
                    settings.agent.context_window = profile.context.default_window;
                }
                eprintln!("profile: {} loaded", profile.name);
            }

            let db_path = data_dir()?.join("sessions.db");
            let store = SessionStore::open(&db_path)?;

            let mcp = build_mcp_manager(&settings).await;

            let (mut agent, summary) = match cli.continue_session {
                Some(prefix) => resume_session(&workspace, &store, prefix.as_deref(), mcp)?,
                None => (Agent::new(settings, workspace, store, mcp)?, None),
            };

            // Apply sampling params from profile to the LLM client
            if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
                agent.apply_model_profile(profile);
            }

            if cli.decoupled_tui {
                app::run_decoupled(agent, summary).await
            } else {
                interactive::run_interactive(agent, summary).await
            }
        }
    }
}

fn cmd_help(topic: &str) -> Result<()> {
    let text = match topic {
        "tools" => include_str!("help/tools.md"),
        "skills" => include_str!("help/skills.md"),
        "config" => include_str!("help/config.md"),
        "commands" => include_str!("help/commands.md"),
        _ => {
            println!("Available topics: tools, skills, config, commands");
            println!("\nUsage: anvil help <topic>");
            return Ok(());
        }
    };
    println!("{text}");
    Ok(())
}

fn cmd_init(workspace: &Path) -> Result<()> {
    let harness = anvil_config::init_harness(workspace)?;
    println!("⚒  Anvil initialized!");
    println!();
    println!("  created: {}", harness.display());
    println!();
    println!("  What's inside:");
    println!("    config.toml   — backend & model settings");
    println!("    models/       — per-model sampling profiles");
    println!("    skills/       — 17 prompt templates (including 3 for kids!)");
    println!("    memory/       — your learned patterns (starts empty)");
    println!();
    println!("  Next steps:");
    println!("    anvil                          start coding!");
    println!("    anvil run -p \"hello world\"     quick one-shot");
    println!();
    println!("  Fun mode for kids:");
    println!("    Type /persona sparkle for Sparkle the Coding Unicorn 🦄");
    println!("    Type /skill kids-first-program to learn coding step by step");
    Ok(())
}

fn cmd_history(limit: usize, search: Option<String>) -> Result<()> {
    let db_path = data_dir()?.join("sessions.db");
    if !db_path.exists() {
        println!("no sessions found");
        return Ok(());
    }
    let store = SessionStore::open(&db_path)?;

    if let Some(query) = search {
        let results = store.search_sessions(&query, limit)?;
        if results.is_empty() {
            println!("no results for '{query}'");
            return Ok(());
        }
        for r in &results {
            let sid = if r.session_id.len() >= 8 {
                &r.session_id[..8]
            } else {
                &r.session_id
            };
            println!(
                "{sid} [{role}] {snippet}",
                role = r.role,
                snippet = r.snippet
            );
        }
        return Ok(());
    }

    let sessions = store.list_sessions(limit)?;

    if sessions.is_empty() {
        println!("no sessions found");
        return Ok(());
    }

    for s in &sessions {
        let title = s.title.as_deref().unwrap_or("(untitled)");
        let date = s.created_at.format("%Y-%m-%d %H:%M");
        println!("{} {} [{}] {}", &s.id[..8], date, s.status, title);
    }
    Ok(())
}

/// Load model profiles from `.anvil/models/` if the harness exists.
fn load_model_profiles(workspace: &Path) -> Vec<anvil_config::ModelProfile> {
    anvil_config::find_harness_dir(workspace)
        .map(|harness| anvil_config::load_profiles(&anvil_config::profiles_dir(&harness)))
        .unwrap_or_default()
}

/// Build an MCP manager from settings. Connects to all configured servers.
/// Returns an empty manager if no servers are configured.
async fn build_mcp_manager(settings: &Settings) -> Arc<McpManager> {
    if settings.mcp.servers.is_empty() {
        return Arc::new(McpManager::empty());
    }

    let configs: Vec<McpServerConfig> = settings
        .mcp
        .servers
        .iter()
        .map(|entry| McpServerConfig {
            name: entry.name.clone(),
            command: entry.command.clone(),
            args: entry.args.clone(),
            env: entry.env.clone(),
        })
        .collect();

    Arc::new(McpManager::new(&configs).await)
}

async fn cmd_run(
    workspace: &Path,
    prompt: &str,
    auto_approve: bool,
    output_format: &str,
) -> Result<()> {
    let mut settings = load_settings(workspace)?;
    auto_detect_model(&mut settings).await;

    let profiles = load_model_profiles(workspace);
    if let Some(profile) = anvil_config::find_matching_profile(&profiles, &settings.provider.model)
    {
        if profile.context.default_window > 0 {
            settings.agent.context_window = profile.context.default_window;
        }
    }

    let db_path = data_dir()?.join("sessions.db");
    let store = SessionStore::open(&db_path)?;
    let mcp = build_mcp_manager(&settings).await;
    let mut agent = Agent::new(settings, workspace.to_path_buf(), store, mcp)?;

    // Apply sampling from profile
    if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
        agent.apply_model_profile(profile);
    }

    let (event_tx, mut event_rx) = mpsc::channel(64);
    let (perm_tx, perm_rx) = mpsc::channel(1);

    if auto_approve {
        tokio::spawn(async move {
            loop {
                if perm_tx.send(PermissionDecision::Allow).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });
    } else {
        tokio::spawn(async move {
            loop {
                eprint!("Allow tool call? [y/n] ");
                let mut input = String::new();
                if std::io::stdin().read_line(&mut input).is_err() {
                    break;
                }
                let decision = match input.trim() {
                    "y" | "Y" | "yes" => PermissionDecision::Allow,
                    "a" | "A" | "always" => PermissionDecision::AllowAlways,
                    _ => PermissionDecision::Deny,
                };
                if perm_tx.send(decision).await.is_err() {
                    break;
                }
            }
        });
    }

    let prompt_owned = prompt.to_string();
    let json_output = output_format == "json";
    let mut final_content = String::new();
    let cancel = CancellationToken::new();

    let turn_cancel = cancel.clone();
    let turn_handle = tokio::spawn(async move {
        agent
            .turn(&prompt_owned, &event_tx, perm_rx, turn_cancel)
            .await
    });

    while let Some(event) = event_rx.recv().await {
        match event {
            AgentEvent::ContentDelta(text) => {
                if json_output {
                    final_content.push_str(&text);
                } else {
                    print!("{text}");
                }
            }
            AgentEvent::ThinkingDelta(_) => {
                // In non-interactive mode, thinking deltas are silently discarded
            }
            AgentEvent::ToolCallPending {
                name, arguments, ..
            } => {
                if !json_output {
                    eprintln!("\n[tool: {name}({arguments})]");
                }
            }
            AgentEvent::ToolResult { name, result } => {
                if !json_output {
                    eprintln!("[{name} result: {} chars]", result.len());
                }
            }
            AgentEvent::Usage(u) => {
                if !json_output {
                    eprintln!(
                        "\n[tokens: {} prompt + {} completion = {} total]",
                        u.prompt_tokens, u.completion_tokens, u.total_tokens
                    );
                }
            }
            AgentEvent::TurnComplete => {
                if !json_output {
                    println!();
                }
            }
            AgentEvent::Retry {
                attempt,
                max,
                delay_secs,
            } => {
                if !json_output {
                    eprintln!("[retrying in {delay_secs:.1}s... (attempt {attempt}/{max})]");
                }
            }
            AgentEvent::LoopDetected { tool_name, count } => {
                if !json_output {
                    eprintln!("[loop detected: {tool_name} x{count}]");
                }
            }
            AgentEvent::ContextWarning {
                estimated_tokens,
                limit,
            } => {
                if !json_output {
                    let pct = (estimated_tokens * 100) / limit;
                    eprintln!("[context: ~{estimated_tokens}/{limit} tokens ({pct}%)]");
                }
            }
            AgentEvent::AutoCompacted {
                messages_removed,
                before_tokens,
                after_tokens,
            } => {
                if !json_output {
                    eprintln!(
                        "[auto-compacted: {messages_removed} messages, ~{before_tokens} → ~{after_tokens} tokens]"
                    );
                }
            }
            AgentEvent::ToolOutputDelta { delta, .. } => {
                if !json_output {
                    eprint!("{delta}");
                }
            }
            AgentEvent::Cancelled => {
                if !json_output {
                    eprintln!("\n[cancelled]");
                }
            }
            AgentEvent::Error(e) => {
                if !json_output {
                    eprintln!("\nerror: {e}");
                }
            }
        }
    }

    turn_handle.await??;

    if json_output {
        let output = serde_json::json!({
            "content": final_content,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    }

    Ok(())
}

/// Run a prompt in autonomous mode (Ralph Loop).
///
/// # How the Ralph Loop works
/// 1. Send the prompt to the LLM with auto-approved tool calls
/// 2. After the turn completes, run the verification command
/// 3. If verify passes (exit 0) → success, print summary, exit
/// 4. If verify fails → feed failure output back as a new user message
/// 5. Repeat until verify passes or limits are hit
///
/// # Why auto-approve is implied
/// The agent can't wait for human permission mid-loop. Autonomous mode
/// is inherently trust-the-agent. Use `--max-iterations` and `--verify`
/// as guardrails instead.
async fn cmd_run_autonomous(
    workspace: &Path,
    prompt: &str,
    verify_cmd: &str,
    max_iterations: usize,
    max_minutes: u64,
) -> Result<()> {
    use anvil_agent::{AutonomousConfig, AutonomousRunner, IterationResult};

    let mut settings = load_settings(workspace)?;
    auto_detect_model(&mut settings).await;

    let profiles = load_model_profiles(workspace);
    if let Some(profile) = anvil_config::find_matching_profile(&profiles, &settings.provider.model)
    {
        if profile.context.default_window > 0 {
            settings.agent.context_window = profile.context.default_window;
        }
    }

    let db_path = data_dir()?.join("sessions.db");
    let store = SessionStore::open(&db_path)?;
    let mcp = build_mcp_manager(&settings).await;
    let mut agent = Agent::new(settings, workspace.to_path_buf(), store, mcp)?;

    if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
        agent.apply_model_profile(profile);
    }

    let config = AutonomousConfig {
        verify_command: verify_cmd.to_string(),
        max_iterations,
        max_tokens: 100_000,
        max_duration: std::time::Duration::from_secs(max_minutes * 60),
    };

    eprintln!(
        "autonomous mode: max {} iterations, {} min, verify: `{}`",
        max_iterations, max_minutes, verify_cmd
    );

    let mut runner = AutonomousRunner::new(config);
    let mut current_prompt = prompt.to_string();

    loop {
        // Check limits before starting next iteration
        let total_tokens = agent.usage().total_tokens;
        if let Some(result) = runner.check_limits(total_tokens) {
            print_autonomous_result(&result, &runner);
            break;
        }

        runner.next_iteration();
        eprintln!(
            "\n--- iteration {}/{} ---",
            runner.iteration(),
            runner.max_iterations()
        );

        // Run a turn with auto-approve
        let (event_tx, mut event_rx) = mpsc::channel(64);
        let (perm_tx, perm_rx) = mpsc::channel(1);

        // Auto-approve all tool calls
        tokio::spawn(async move {
            loop {
                if perm_tx.send(PermissionDecision::Allow).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        let prompt_clone = current_prompt.clone();
        let turn_cancel = CancellationToken::new();
        let turn_handle = tokio::spawn(async move {
            let result = agent
                .turn(&prompt_clone, &event_tx, perm_rx, turn_cancel)
                .await;
            result.map(|()| agent)
        });

        let mut turn_content = String::new();
        while let Some(event) = event_rx.recv().await {
            match event {
                AgentEvent::ContentDelta(text) => {
                    print!("{text}");
                    turn_content.push_str(&text);
                }
                AgentEvent::ToolCallPending {
                    name, arguments, ..
                } => {
                    eprintln!("\n[tool: {name}({arguments})]");
                }
                AgentEvent::ToolResult { name, result } => {
                    eprintln!("[{name} result: {} chars]", result.len());
                }
                AgentEvent::Error(e) => {
                    eprintln!("\nerror: {e}");
                }
                _ => {}
            }
        }

        agent = turn_handle.await??;

        // Check for LLM DONE marker
        if anvil_agent::autonomous::contains_done_marker(&turn_content) {
            eprintln!("\n[LLM declared DONE — running final verification]");
        }

        // Run verification
        eprintln!("\n[verifying: `{}`]", runner.verify_command());
        let result = runner.run_verify();

        match &result {
            IterationResult::VerifyPassed { stdout } => {
                eprintln!("[PASS] {}", stdout.trim());
                print_autonomous_result(&result, &runner);
                break;
            }
            IterationResult::VerifyFailed {
                stdout,
                stderr,
                exit_code,
            } => {
                eprintln!("[FAIL] exit code {exit_code}");
                // Feed failure back to the LLM for the next iteration
                let feedback = format!(
                    "The verification command `{}` failed (exit code {}).\n\
                     stdout:\n{}\nstderr:\n{}\n\
                     Please fix the issue and try again.",
                    runner.verify_command(),
                    exit_code,
                    stdout.trim(),
                    stderr.trim()
                );
                current_prompt = feedback;
            }
            _ => {
                print_autonomous_result(&result, &runner);
                break;
            }
        }
    }

    Ok(())
}

/// Print a summary of the autonomous run result.
fn print_autonomous_result(
    result: &anvil_agent::IterationResult,
    runner: &anvil_agent::AutonomousRunner,
) {
    let elapsed = runner.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;

    match result {
        anvil_agent::IterationResult::VerifyPassed { .. } => {
            eprintln!(
                "\nautonomous: PASSED after {} iterations ({mins}m {secs}s)",
                runner.iteration()
            );
        }
        anvil_agent::IterationResult::MaxIterationsReached => {
            eprintln!(
                "\nautonomous: STOPPED — max iterations ({}) reached ({mins}m {secs}s)",
                runner.max_iterations()
            );
        }
        anvil_agent::IterationResult::MaxTokensReached => {
            eprintln!("\nautonomous: STOPPED — token budget exceeded ({mins}m {secs}s)");
        }
        anvil_agent::IterationResult::TimeoutReached => {
            eprintln!("\nautonomous: STOPPED — time limit reached ({mins}m {secs}s)");
        }
        anvil_agent::IterationResult::VerifyFailed { exit_code, .. } => {
            eprintln!("\nautonomous: FAILED — verify exit code {exit_code} ({mins}m {secs}s)");
        }
        anvil_agent::IterationResult::LlmDeclaredDone => {
            eprintln!("\nautonomous: LLM declared done ({mins}m {secs}s)");
        }
    }
}

/// Query the backend for available models. If the configured model isn't available,
/// switch to the first available model.
///
/// # How backend detection works
/// - **Ollama**: queries `/api/tags` (Ollama-specific endpoint)
/// - **llama-server / MLX**: queries `/v1/models` (OpenAI standard)
/// - **Custom**: skips detection entirely (user manages their own backend)
///
/// This runs at startup before the first LLM call. If the backend is unreachable,
/// we silently continue with the configured model — the error will surface on
/// the first actual API call with a clearer message.
async fn auto_detect_model(settings: &mut anvil_config::Settings) {
    use anvil_config::BackendKind;

    let models = match settings.provider.backend {
        BackendKind::Ollama => query_ollama_models(&settings.provider.base_url).await,
        BackendKind::LlamaServer | BackendKind::Mlx => {
            query_openai_models(&settings.provider.base_url).await
        }
        BackendKind::Custom => return,
    };

    let models = match models {
        Some(m) if !m.is_empty() => m,
        _ => return,
    };

    let configured = &settings.provider.model;
    let is_available = models
        .iter()
        .any(|m| m == configured || m.starts_with(&format!("{configured}:")));

    if !is_available {
        let first = &models[0];
        eprintln!(
            "model '{}' not found on {}, using '{}'",
            configured, settings.provider.backend, first
        );
        settings.provider.model = first.clone();
    }
}

/// Query Ollama's proprietary `/api/tags` endpoint for model names.
async fn query_ollama_models(base_url: &str) -> Option<Vec<String>> {
    let base = base_url.trim_end_matches("/v1");
    let url = format!("{base}/api/tags");
    let resp = reqwest::get(&url).await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    body["models"].as_array().map(|arr| {
        arr.iter()
            .filter_map(|m| m["name"].as_str().map(String::from))
            .collect()
    })
}

/// Query the standard OpenAI `/v1/models` endpoint (used by llama-server, MLX).
async fn query_openai_models(base_url: &str) -> Option<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = reqwest::get(&url).await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;
    body["data"].as_array().map(|arr| {
        arr.iter()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect()
    })
}

fn resume_session(
    workspace: &Path,
    store: &SessionStore,
    prefix: Option<&str>,
    mcp: Arc<McpManager>,
) -> Result<(Agent, Option<String>)> {
    let session = match prefix {
        Some(p) if !p.is_empty() => store
            .find_by_prefix(p)?
            .ok_or_else(|| anyhow::anyhow!("no session found matching '{p}'"))?,
        _ => store
            .find_latest_resumable()?
            .ok_or_else(|| anyhow::anyhow!("no resumable session found"))?,
    };

    let settings = load_settings(workspace)?;
    let messages = store.load_messages(&session.id)?;
    let msg_count = messages.len();
    let summary = format!(
        "Resuming session {} ({} messages, started {})",
        &session.id[..8],
        msg_count,
        session.created_at.format("%Y-%m-%d %H:%M")
    );

    let agent = Agent::resume(
        settings,
        workspace.to_path_buf(),
        store.clone(),
        &session.id,
        messages,
        mcp,
    )?;

    Ok((agent, Some(summary)))
}
