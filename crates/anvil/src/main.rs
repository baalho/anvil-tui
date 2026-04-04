mod backend;
mod client;
mod commands;
mod daemon;
mod interactive;
mod ipc;
pub mod render;
mod watcher;

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

    /// Launch profile name (bundles persona + mode + skills + model)
    #[arg(short = 'p', long = "profile")]
    profile: Option<String>,

    /// Launch inside a Zellij session with a bundled layout
    #[arg(long = "zellij")]
    zellij: Option<Option<String>>,
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
    /// Watch workspace for file changes and react automatically
    Watch {
        /// Glob patterns to ignore (substring match)
        #[arg(short, long)]
        ignore: Vec<String>,
        /// Debounce interval in seconds
        #[arg(long, default_value = "2")]
        debounce: u64,
    },
    /// Manage the background daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Send a prompt to the running daemon
    Send {
        /// The prompt text
        prompt: String,
        /// Auto-approve all tool calls
        #[arg(short = 'y', long = "yes")]
        auto_approve: bool,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon (runs in foreground — use nohup/systemd to background)
    Start,
    /// Stop the running daemon
    Stop,
    /// Show daemon status
    Status,
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

    // Handle --zellij before anything else
    if let Some(ref zellij_layout) = cli.zellij {
        return launch_zellij(&workspace, zellij_layout.as_deref());
    }

    match cli.command {
        Some(Commands::Init) => cmd_init(&workspace),
        Some(Commands::History { limit, search }) => cmd_history(limit, search),
        Some(Commands::Watch { ignore, debounce }) => cmd_watch(&workspace, debounce, ignore).await,
        Some(Commands::Daemon { action }) => match action {
            DaemonAction::Start => cmd_daemon_start(&workspace).await,
            DaemonAction::Stop => client::daemon_stop(&workspace).await,
            DaemonAction::Status => client::daemon_status(&workspace).await,
        },
        Some(Commands::Send {
            prompt,
            auto_approve,
        }) => {
            let code = client::send_prompt(&workspace, &prompt, auto_approve).await?;
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
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
                // Use effective_context() which respects kv_cache.recommended_context
                let effective = profile.effective_context();
                if effective > 0 {
                    settings.agent.context_window = effective;
                }
                eprintln!("profile: {} loaded", profile.name);
                if let Some(ref kv) = profile.kv_cache {
                    eprintln!(
                        "  KV cache: K={} V={} | context: {} tokens",
                        kv.type_k, kv.type_v, kv.recommended_context
                    );
                }
            }

            let db_path = data_dir()?.join("sessions.db");
            let store = SessionStore::open(&db_path)?;

            let mcp = build_mcp_manager(&settings).await;

            let (mut agent, summary) = match cli.continue_session {
                Some(prefix) => resume_session(&workspace, &store, prefix.as_deref(), mcp)?,
                None => (Agent::new(settings.clone(), workspace, store, mcp)?, None),
            };

            // Apply sampling params from profile to the LLM client
            if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
                agent.apply_model_profile(profile);
            }

            // Apply launch profile if --profile was given
            if let Some(profile_name) = &cli.profile {
                // workspace was moved into Agent::new — retrieve it back via the agent
                let ws = agent.workspace().to_path_buf();
                apply_launch_profile(&mut agent, &settings, profile_name, &ws)?;
                // Remember this profile for next time
                let _ = anvil_config::save_last_profile(profile_name);
            }

            // Warn if Ollama backend without OLLAMA_NUM_CTX set
            if matches!(agent.backend(), anvil_config::BackendKind::Ollama)
                && std::env::var("OLLAMA_NUM_CTX").is_err()
            {
                eprintln!("  ⚠ Ollama defaults to 2048 context. Set OLLAMA_NUM_CTX=8192 or use a model profile.");
            }

            interactive::run_interactive(agent, summary).await
        }
    }
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
    println!("    layouts/      — Zellij terminal layouts (TQ, dev, ops)");
    println!("    memory/       — your learned patterns (starts empty)");
    println!();
    println!("  Next steps:");
    println!("    anvil                          start coding!");
    println!("    anvil --zellij anvil-tq        TurboQuant layout (llama-server + anvil)");
    println!("    anvil run -p \"hello world\"     quick one-shot");
    println!();
    println!("  Fun mode for kids:");
    println!("    Type /persona sparkle — activates Sparkle + kids mode automatically!");
    println!("    Then just tell Sparkle what you like and watch the magic happen 🦄");
    Ok(())
}

/// Launch Anvil inside a Zellij session with a bundled layout.
///
/// Resolves the layout from `.anvil/layouts/<name>.kdl`. If already inside
/// Zellij (`$ZELLIJ` env var set), prints a warning and exits — no nested
/// sessions. Default layout is `anvil-dev` if no name given.
fn launch_zellij(workspace: &Path, layout_name: Option<&str>) -> Result<()> {
    // Don't nest Zellij sessions
    if std::env::var("ZELLIJ").is_ok() {
        eprintln!("  ⚠ Already inside a Zellij session. Run anvil directly.");
        return Ok(());
    }

    // Skip Zellij inside devcontainers (no terminal multiplexer)
    if anvil_agent::system_prompt::detect_devcontainer(workspace).is_some() {
        eprintln!("  ⚠ Inside a devcontainer — skipping Zellij launch. Run anvil directly.");
        return Ok(());
    }

    let name = layout_name.unwrap_or("anvil-dev");
    let layout_filename = if name.ends_with(".kdl") {
        name.to_string()
    } else {
        format!("{name}.kdl")
    };

    // Look for layout in .anvil/layouts/ first, then try as absolute path
    let layout_path = if let Some(harness) = anvil_config::find_harness_dir(workspace) {
        let candidate = harness.join("layouts").join(&layout_filename);
        if candidate.exists() {
            candidate
        } else {
            let abs = PathBuf::from(&layout_filename);
            if abs.exists() {
                abs
            } else {
                anyhow::bail!(
                    "layout '{}' not found. Run `anvil init` to create bundled layouts, \
                     or check .anvil/layouts/",
                    name
                );
            }
        }
    } else {
        anyhow::bail!("no .anvil/ directory found. Run `anvil init` first.");
    };

    // Derive session name from layout filename (without .kdl extension)
    let session_name = layout_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("anvil");

    eprintln!("  launching Zellij layout: {}", layout_path.display());
    eprintln!("  session: {session_name}");

    // exec into Zellij — replaces this process
    let status = std::process::Command::new("zellij")
        .arg("--layout")
        .arg(&layout_path)
        .arg("--session")
        .arg(session_name)
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => anyhow::bail!("zellij exited with {}", s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "zellij not found. Install it: https://zellij.dev/documentation/installation"
            );
        }
        Err(e) => anyhow::bail!("failed to launch zellij: {e}"),
    }
}

/// Apply a named launch profile — sets persona, mode, skills, and model in one shot.
fn apply_launch_profile(
    agent: &mut Agent,
    settings: &Settings,
    profile_name: &str,
    workspace: &Path,
) -> Result<()> {
    let profile = settings
        .profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(profile_name))
        .ok_or_else(|| {
            let available: Vec<&str> = settings.profiles.iter().map(|p| p.name.as_str()).collect();
            if available.is_empty() {
                anyhow::anyhow!(
                    "no profile '{}' found. Add [[profiles]] to .anvil/config.toml",
                    profile_name
                )
            } else {
                anyhow::anyhow!(
                    "no profile '{}'. available: {}",
                    profile_name,
                    available.join(", ")
                )
            }
        })?;

    // Apply model if specified
    if !profile.model.is_empty() {
        agent.set_model(profile.model.clone());
        // Check workspace model profiles first (user's custom .anvil/models/ files),
        // then fall back to bundled profiles. Workspace profiles take priority so
        // users can tune sampling for their specific hardware without touching
        // bundled defaults.
        let workspace_profiles = load_model_profiles(workspace);
        let bundled_profiles = anvil_config::load_bundled_profiles();
        let mp = anvil_config::find_matching_profile(&workspace_profiles, &profile.model)
            .or_else(|| anvil_config::find_matching_profile(&bundled_profiles, &profile.model));
        if let Some(mp) = mp {
            agent.apply_model_profile(mp);
        }
    }

    // Apply per-profile base_url if set — allows routing different profiles
    // to different backend servers (e.g. kids on :8081, coding on :8080).
    // Preserves the current backend kind; only the URL changes.
    if !profile.base_url.is_empty() {
        let kind = agent.backend().clone();
        agent.set_backend(kind, profile.base_url.clone());
    }

    // Apply persona if specified
    if !profile.persona.is_empty() {
        if let Some(persona) = anvil_agent::find_persona(&profile.persona) {
            agent.set_persona(Some(persona));
        } else {
            eprintln!(
                "  ⚠ profile '{}': unknown persona '{}'",
                profile.name, profile.persona
            );
        }
    }

    // Apply mode if specified (overrides persona's default)
    if !profile.mode.is_empty() {
        match profile.mode.to_lowercase().as_str() {
            "coding" | "code" => agent.set_mode(anvil_agent::Mode::Coding),
            "creative" | "create" => agent.set_mode(anvil_agent::Mode::Creative),
            _ => eprintln!(
                "  ⚠ profile '{}': unknown mode '{}'",
                profile.name, profile.mode
            ),
        }
    }

    // Activate skills
    for skill_key in &profile.skills {
        let loader = anvil_agent::SkillLoader::new(agent.workspace());
        let skills: Vec<anvil_agent::Skill> = loader.scan();
        if let Some(skill) = skills
            .into_iter()
            .find(|s| s.key.eq_ignore_ascii_case(skill_key))
        {
            agent.activate_skill(skill);
        } else {
            eprintln!(
                "  ⚠ profile '{}': unknown skill '{}'",
                profile.name, skill_key
            );
        }
    }

    eprintln!("  profile: {} loaded", profile.name);
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
                    eprintln!("[{name} result: {} chars]", result.text().len());
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
                    eprintln!("[{name} result: {} chars]", result.text().len());
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

/// Watch workspace for file changes and trigger agent turns.
///
/// Blocks the terminal until Ctrl+C. The file watcher runs in a background
/// thread, debounces events, and sends them through the Event abstraction.
/// In v2.0, this same watcher runs inside the daemon — the code is identical.
async fn cmd_watch(workspace: &Path, debounce_secs: u64, ignore: Vec<String>) -> Result<()> {
    use anvil_agent::{dispatch_event, DispatchResult, Event};

    let mut settings = load_settings(workspace)?;
    auto_detect_model(&mut settings).await;

    let profiles = load_model_profiles(workspace);
    if let Some(profile) = anvil_config::find_matching_profile(&profiles, &settings.provider.model)
    {
        let effective = profile.effective_context();
        if effective > 0 {
            settings.agent.context_window = effective;
        }
    }

    let db_path = data_dir()?.join("sessions.db");
    let store = SessionStore::open(&db_path)?;
    let mcp = build_mcp_manager(&settings).await;
    let mut agent = Agent::new(settings, workspace.to_path_buf(), store, mcp)?;

    if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
        agent.apply_model_profile(profile);
    }

    // Create write ledger — shared between tool executor and file watcher
    // to prevent the agent's own file writes from triggering new turns.
    let ledger = anvil_tools::WriteLedger::new();
    agent.set_write_ledger(ledger.clone());

    eprintln!("╭─────────────────────────────────────╮");
    eprintln!("│  ⚒  Anvil Watch v{:<19}│", env!("CARGO_PKG_VERSION"));
    eprintln!("│  watching for file changes...        │");
    eprintln!("╰─────────────────────────────────────╯");
    eprintln!("  model:    {}", agent.model());
    eprintln!("  session:  {}", &agent.session_id()[..8]);
    eprintln!("  cwd:      {}", workspace.display());
    eprintln!("  debounce: {}s", debounce_secs);
    eprintln!("  press Ctrl+C to stop");
    eprintln!();

    // Event channel — the v2.0 bridge
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(32);

    // Spawn the file watcher in a blocking thread (notify uses std::sync)
    let watcher_tx = event_tx.clone();
    let watch_workspace = workspace.to_path_buf();
    let watcher_ledger = ledger.clone();
    let watcher_handle = tokio::task::spawn_blocking(move || {
        let config = watcher::WatchConfig {
            workspace: watch_workspace,
            debounce: std::time::Duration::from_secs(debounce_secs),
            ignore_patterns: ignore,
            write_ledger: Some(watcher_ledger),
        };
        if let Err(e) = watcher::run_file_watcher(config, watcher_tx) {
            tracing::error!("file watcher failed: {e}");
        }
    });

    // Spawn signal handler
    let signal_tx = event_tx.clone();
    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            eprintln!("\n  shutting down...");
            let _ = signal_tx.send(Event::Shutdown).await;
        }
    });

    // Drop our copy so the loop exits when all producers drop
    drop(event_tx);

    let renderer = crate::render::TerminalRenderer::new();

    // Dispatch loop — process events until shutdown
    while let Some(event) = event_rx.recv().await {
        // Log what triggered this turn
        let trigger_desc = match &event {
            Event::FileChanged { paths } => {
                let names: Vec<&str> = paths
                    .iter()
                    .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                    .take(5)
                    .collect();
                format!("{} file(s): {}", paths.len(), names.join(", "))
            }
            Event::Shutdown => "shutdown".into(),
            Event::UserPrompt { .. } => "prompt".into(),
        };
        eprintln!("  ⚡ {trigger_desc}");

        // Create channels for this turn
        let (agent_event_tx, mut agent_event_rx) = mpsc::channel::<AgentEvent>(64);
        let (_perm_tx, perm_rx) = mpsc::channel::<PermissionDecision>(1);
        // Watch mode auto-approves all tool calls — user isn't at the keyboard.
        // The permission_rx channel is immediately dropped, which the agent
        // treats as auto-approve for read-only tools and deny for mutating ones.

        let cancel = CancellationToken::new();

        // Drain agent events to terminal in a background task
        let drain_handle = tokio::spawn(async move {
            use crate::render::Renderer;
            let r = crate::render::TerminalRenderer::new();
            while let Some(ev) = agent_event_rx.recv().await {
                match ev {
                    AgentEvent::ContentDelta(text) => r.render_content_delta(&text),
                    AgentEvent::ThinkingDelta(text) => r.render_thinking_delta(&text),
                    AgentEvent::ToolCallPending {
                        name, arguments, ..
                    } => {
                        let short = if arguments.len() > 60 {
                            format!("{}...", &arguments[..60])
                        } else {
                            arguments
                        };
                        r.render_tool_pending(&name, "⚙", &short);
                    }
                    AgentEvent::ToolResult { name, result } => {
                        let text = result.text();
                        r.render_tool_result(&name, "✓", text);
                    }
                    AgentEvent::Error(msg) => r.render_error(&msg),
                    AgentEvent::TurnComplete => {
                        r.render_info("  [turn complete]");
                    }
                    _ => {} // Other events logged at trace level
                }
            }
        });

        let result = dispatch_event(&mut agent, event, &agent_event_tx, perm_rx, cancel).await?;

        // Drop the sender so the drain task finishes
        drop(agent_event_tx);
        let _ = drain_handle.await;

        match result {
            DispatchResult::Shutdown => break,
            DispatchResult::Handled => {
                eprintln!();
            }
        }
    }

    watcher_handle.abort();
    let _ = renderer; // suppress unused warning
    eprintln!(
        "  watch session ended. session: {}",
        &agent.session_id()[..8]
    );
    agent.pause_session()?;

    Ok(())
}

/// Start the daemon server.
///
/// Runs in the foreground. To background it, use standard Unix tools:
/// - `nohup anvil daemon start &`
/// - systemd service
/// - launchd plist
/// - tmux/screen/zellij
///
/// This follows the "boring over clever" principle — we don't reinvent
/// process supervision when the OS already provides it.
async fn cmd_daemon_start(workspace: &Path) -> Result<()> {
    let mut settings = load_settings(workspace)?;
    auto_detect_model(&mut settings).await;

    let profiles = load_model_profiles(workspace);
    if let Some(profile) = anvil_config::find_matching_profile(&profiles, &settings.provider.model)
    {
        let effective = profile.effective_context();
        if effective > 0 {
            settings.agent.context_window = effective;
        }
    }

    let db_path = data_dir()?.join("sessions.db");
    let store = SessionStore::open(&db_path)?;
    let mcp = build_mcp_manager(&settings).await;
    let mut agent = Agent::new(settings, workspace.to_path_buf(), store, mcp)?;

    if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
        agent.apply_model_profile(profile);
    }

    daemon::run_daemon(agent).await
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
