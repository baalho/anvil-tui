use anvil_agent::{Agent, SkillLoader};
use anvil_llm::TokenUsage;

pub enum CommandResult {
    Handled(String),
    Exit,
    Unknown(String),
}

pub async fn handle_command(
    input: &str,
    agent: &mut Agent,
    cumulative_usage: &TokenUsage,
) -> CommandResult {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

    match cmd {
        "/help" => CommandResult::Handled(help_text()),
        "/end" => {
            if let Err(e) = agent.end_session() {
                return CommandResult::Handled(format!("error ending session: {e}"));
            }
            CommandResult::Exit
        }
        "/stats" => CommandResult::Handled(stats_text(agent, cumulative_usage)),
        "/model" => CommandResult::Handled(model_command(agent, arg).await),
        "/backend" => CommandResult::Handled(backend_command(agent, arg)),
        "/history" => CommandResult::Handled(history_text(agent)),
        "/ralph" => CommandResult::Handled(ralph_help(arg)),
        "/clear" => CommandResult::Handled("context compaction not yet implemented".to_string()),
        "/skill" => CommandResult::Handled(skill_command(agent, arg)),
        _ => CommandResult::Unknown(cmd.to_string()),
    }
}

fn help_text() -> String {
    [
        "Available commands:",
        "  /help                        Show this help",
        "  /stats                       Token usage and session info",
        "  /model [name]                Show or switch model",
        "  /backend [type url]          Show or switch backend (ollama|llama|mlx|custom)",
        "  /history                     List recent sessions",
        "  /skill [name]                List or activate a skill",
        "  /skill clear                 Deactivate all skills",
        "  /skill verify <name>         Run a skill's verification command",
        "  /ralph <prompt> --verify <cmd> Run autonomous mode (Ralph Loop)",
        "  /clear                       Compact conversation context",
        "  /end                         End session and exit",
    ]
    .join("\n")
}

fn stats_text(agent: &Agent, usage: &TokenUsage) -> String {
    let u = usage;
    let msg_count = agent.messages().len();
    let mut text = format!(
        "session:     {}\n\
         model:       {}\n\
         backend:     {}\n\
         messages:    {msg_count}\n\
         tokens:      {} prompt + {} completion = {} total\n\
         cost:        ${:.4}",
        &agent.session_id()[..8],
        agent.model(),
        agent.backend(),
        u.prompt_tokens,
        u.completion_tokens,
        u.total_tokens,
        u.estimated_cost_usd.unwrap_or(0.0),
    );

    let extra = agent.extra_env();
    if !extra.is_empty() {
        text.push_str(&format!("\nenv pass:    {}", extra.join(", ")));
    }

    text
}

async fn model_command(agent: &mut Agent, arg: &str) -> String {
    if arg.is_empty() {
        return format!("current model: {}", agent.model());
    }

    // Query Ollama for available models
    let base = agent.base_url().trim_end_matches("/v1");
    let url = format!("{base}/api/tags");

    match reqwest::get(&url).await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(body) => {
                let models = body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if models
                    .iter()
                    .any(|m| m == arg || m.starts_with(&format!("{arg}:")))
                {
                    let model_name = if models.contains(&arg.to_string()) {
                        arg.to_string()
                    } else {
                        models
                            .iter()
                            .find(|m| m.starts_with(&format!("{arg}:")))
                            .cloned()
                            .unwrap_or_else(|| arg.to_string())
                    };
                    agent.set_model(model_name.clone());
                    format!("switched to model: {model_name}")
                } else {
                    let available = models.join(", ");
                    format!("model '{arg}' not found. available: {available}")
                }
            }
            Err(e) => format!("failed to parse Ollama response: {e}"),
        },
        Err(e) => format!("failed to connect to Ollama: {e}"),
    }
}

/// Handle `/ralph` — show usage info for autonomous mode.
///
/// The actual Ralph Loop execution happens in interactive.rs when the user
/// provides a prompt with --verify. This command just shows help.
fn ralph_help(arg: &str) -> String {
    if arg.is_empty() {
        return [
            "Ralph Loop (autonomous mode):",
            "",
            "Usage from CLI:",
            "  anvil run -p 'fix all tests' -a --verify 'cargo test'",
            "  anvil run -p 'deploy stack' -a --verify 'docker compose ps' --max-iterations 5",
            "",
            "The agent runs your prompt, then checks the verify command.",
            "If verify fails, the failure output is fed back for another attempt.",
            "Stops when verify passes or limits are hit.",
        ]
        .join("\n");
    }
    "ralph loop is available via: anvil run -p '<prompt>' -a --verify '<cmd>'".to_string()
}

/// Handle `/backend` — show or switch the inference backend.
///
/// # Usage
/// - `/backend` — show current backend type and URL
/// - `/backend ollama http://localhost:11434/v1` — switch to Ollama
/// - `/backend llama http://localhost:8080/v1` — switch to llama-server
/// - `/backend mlx http://localhost:8080/v1` — switch to MLX
///
/// # Why this matters
/// Different models work better with different backends. GLM-4.7-Flash has
/// chat template bugs on Ollama but works perfectly on llama-server.
/// This command lets users switch without restarting Anvil.
fn backend_command(agent: &mut Agent, arg: &str) -> String {
    if arg.is_empty() {
        return format!(
            "backend: {}\nurl:     {}",
            agent.backend(),
            agent.base_url()
        );
    }

    let parts: Vec<&str> = arg.splitn(2, ' ').collect();
    let backend_str = parts[0];
    let url = parts.get(1).map(|s| s.trim());

    let backend = match backend_str {
        "ollama" => anvil_config::BackendKind::Ollama,
        "llama" | "llama-server" => anvil_config::BackendKind::LlamaServer,
        "mlx" => anvil_config::BackendKind::Mlx,
        "custom" => anvil_config::BackendKind::Custom,
        _ => {
            return format!(
                "unknown backend '{}'. options: ollama, llama, mlx, custom",
                backend_str
            )
        }
    };

    let base_url = match url {
        Some(u) => u.to_string(),
        None => match backend {
            anvil_config::BackendKind::Ollama => "http://localhost:11434/v1".to_string(),
            anvil_config::BackendKind::LlamaServer => "http://localhost:8080/v1".to_string(),
            anvil_config::BackendKind::Mlx => "http://localhost:8080/v1".to_string(),
            anvil_config::BackendKind::Custom => {
                return "custom backend requires a URL: /backend custom http://host:port/v1"
                    .to_string()
            }
        },
    };

    agent.set_backend(backend.clone(), base_url.clone());
    format!("switched to {} at {}", backend, base_url)
}

/// Handle `/skill` — list, activate, deactivate, or verify skills.
///
/// # Subcommands
/// - `/skill` — list all skills, grouped by category
/// - `/skill <name>` — activate a skill (injects into system prompt + enables env vars)
/// - `/skill clear` — deactivate all skills
/// - `/skill verify <name>` — run a skill's verification command
fn skill_command(agent: &mut Agent, arg: &str) -> String {
    let loader = SkillLoader::new(agent.workspace());

    if arg.is_empty() {
        return list_skills(&loader, agent);
    }

    if arg == "clear" {
        agent.clear_skills();
        return "all skills deactivated".to_string();
    }

    // Handle `/skill verify <name>`
    if let Some(skill_name) = arg.strip_prefix("verify ") {
        let skill_name = skill_name.trim();
        return verify_skill(&loader, skill_name);
    }

    match loader.get(arg) {
        Ok(skill) => {
            let env_info = if skill.required_env.is_empty() {
                String::new()
            } else {
                format!(" (env: {})", skill.required_env.join(", "))
            };
            agent.activate_skill(skill);
            format!("skill '{arg}' activated{env_info}")
        }
        Err(e) => format!("{e}"),
    }
}

/// List all skills, grouped by category when categories are present.
///
/// # Why group by category
/// With 14+ skills, a flat list becomes hard to scan. Categories (infrastructure,
/// dev-tools, meta) give users a mental model of what's available.
fn list_skills(loader: &SkillLoader, agent: &Agent) -> String {
    let skills = loader.scan();
    if skills.is_empty() {
        return "no skills found in .anvil/skills/".to_string();
    }

    // Group by category
    let mut categorized: std::collections::BTreeMap<String, Vec<&anvil_agent::Skill>> =
        std::collections::BTreeMap::new();
    let mut uncategorized: Vec<&anvil_agent::Skill> = Vec::new();

    for skill in &skills {
        match &skill.category {
            Some(cat) => categorized.entry(cat.clone()).or_default().push(skill),
            None => uncategorized.push(skill),
        }
    }

    let mut output = String::from("Available skills:\n");

    for (category, cat_skills) in &categorized {
        output.push_str(&format!("\n  [{category}]\n"));
        for skill in cat_skills {
            let active = if agent.has_active_skill(&skill.key) {
                " (active)"
            } else {
                ""
            };
            output.push_str(&format!(
                "    {} — {}{}\n",
                skill.key, skill.description, active
            ));
        }
    }

    if !uncategorized.is_empty() {
        if !categorized.is_empty() {
            output.push_str("\n  [other]\n");
        }
        for skill in &uncategorized {
            let active = if agent.has_active_skill(&skill.key) {
                " (active)"
            } else {
                ""
            };
            output.push_str(&format!(
                "    {} — {}{}\n",
                skill.key, skill.description, active
            ));
        }
    }

    output.push_str("\nUse /skill <name> to activate, /skill clear to deactivate all.");
    output.push_str("\nUse /skill verify <name> to check prerequisites.");
    output
}

/// Run a skill's verification command and report pass/fail.
///
/// # How verification works
/// Each skill can declare a `verify` command in its frontmatter:
/// ```yaml
/// verify: "docker info --format '{{.ServerVersion}}'"
/// ```
/// This command is run via `sh -c` (or `cmd.exe /C` on Windows).
/// Exit 0 = prerequisites met. Non-zero = something is missing.
fn verify_skill(loader: &SkillLoader, name: &str) -> String {
    let skill = match loader.get(name) {
        Ok(s) => s,
        Err(e) => return format!("{e}"),
    };

    let cmd_str = match &skill.verify_command {
        Some(cmd) => cmd,
        None => return format!("skill '{name}' has no verify command"),
    };

    // Run the verify command synchronously (it should be fast)
    #[cfg(unix)]
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd_str)
        .output();

    #[cfg(windows)]
    let output = std::process::Command::new("cmd.exe")
        .arg("/C")
        .arg(cmd_str)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() {
                let detail = stdout.trim();
                if detail.is_empty() {
                    format!("verify '{name}': PASS")
                } else {
                    format!("verify '{name}': PASS\n  {detail}")
                }
            } else {
                let detail = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    stdout.trim().to_string()
                };
                format!(
                    "verify '{name}': FAIL (exit {})\n  {detail}",
                    out.status.code().unwrap_or(-1)
                )
            }
        }
        Err(e) => format!("verify '{name}': ERROR — {e}"),
    }
}

fn history_text(agent: &Agent) -> String {
    // Reuse the session listing logic
    let db_path = match anvil_config::data_dir() {
        Ok(d) => d.join("sessions.db"),
        Err(e) => return format!("error: {e}"),
    };
    let store = match anvil_agent::SessionStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => return format!("error: {e}"),
    };
    let sessions = match store.list_sessions(10) {
        Ok(s) => s,
        Err(e) => return format!("error: {e}"),
    };

    if sessions.is_empty() {
        return "no sessions found".to_string();
    }

    let current_id = agent.session_id();
    sessions
        .iter()
        .map(|s| {
            let marker = if s.id == current_id {
                " ← current"
            } else {
                ""
            };
            let title = s.title.as_deref().unwrap_or("(untitled)");
            let date = s.created_at.format("%Y-%m-%d %H:%M");
            format!("{} {} [{}] {}{}", &s.id[..8], date, s.status, title, marker)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
