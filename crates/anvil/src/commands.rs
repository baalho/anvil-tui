use anvil_agent::{Agent, MemoryStore, SkillLoader};
use anvil_llm::TokenUsage;

pub enum CommandResult {
    Handled(String),
    /// Trigger context compaction (requires async agent interaction).
    Compact,
    /// Start an interactive Ralph Loop with the given config.
    Ralph(RalphArgs),
    /// Start a managed backend process.
    BackendStart(BackendStartArgs),
    /// Stop the managed backend process.
    BackendStop,
    Exit,
    Unknown(String),
}

/// Arguments for starting a managed backend.
pub struct BackendStartArgs {
    pub backend_type: String,
    pub model_path: String,
    pub port: u16,
}

/// Parsed arguments for an interactive `/ralph` command.
pub struct RalphArgs {
    pub prompt: String,
    pub verify_command: String,
    pub max_iterations: usize,
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
        "/backend" => backend_command(agent, arg),
        "/history" => CommandResult::Handled(history_text(agent)),
        "/ralph" => ralph_command(arg),
        "/clear" => CommandResult::Compact,
        "/think" => CommandResult::Handled(think_command(agent)),
        "/memory" => CommandResult::Handled(memory_command(agent, arg)),
        "/route" => CommandResult::Handled(route_command(agent, arg)),
        "/skill" => CommandResult::Handled(skill_command(agent, arg)),
        "/mcp" => CommandResult::Handled(mcp_command(agent, arg).await),
        "/persona" => CommandResult::Handled(persona_command(agent, arg)),
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
        "  /backend start llama <model>  Start a managed llama-server",
        "  /backend stop                Stop the managed backend",
        "  /history                     List recent sessions",
        "  /skill [name]                List or activate a skill",
        "  /skill clear                 Deactivate all skills",
        "  /skill verify <name>         Run a skill's verification command",
        "  /ralph <prompt> --verify <cmd> Run autonomous mode (Ralph Loop)",
        "  /clear                       Compact conversation context",
        "  /think                       Toggle <think> block visibility",
        "  /route [tool model]          Show or set model routing",
        "  /memory                      List stored patterns",
        "  /memory                      List stored patterns",
        "  /memory add <pattern>        Save a new pattern",
        "  /memory add category:<t> <p> Save with category (convention|gotcha|pattern)",
        "  /memory search <keyword>     Search memories",
        "  /memory rm <filename>        Remove one entry",
        "  /memory clear                Remove all patterns",
        "  /mcp                         List MCP servers and tools",
        "  /persona [name]              List or activate a character persona",
        "  /persona clear               Deactivate persona",
        "  /end                         End session and exit",
    ]
    .join("\n")
}

fn route_command(agent: &mut Agent, arg: &str) -> String {
    if arg.is_empty() {
        let routes = agent.router().routes();
        if routes.is_empty() {
            return "no routes configured. usage: /route <tool> <model>".to_string();
        }
        let mut output = String::from("model routes:\n");
        for (tool, model) in routes {
            output.push_str(&format!("  {tool} → {model}\n"));
        }
        return output;
    }

    if arg == "clear" {
        *agent.router_mut() = anvil_agent::ModelRouter::new();
        return "all routes cleared".to_string();
    }

    let parts: Vec<&str> = arg.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return "usage: /route <tool> <model> or /route clear".to_string();
    }

    let tool = parts[0];
    let model = parts[1].trim();
    agent.router_mut().add_route(tool, model);
    format!("{tool} → {model}")
}

fn memory_command(agent: &Agent, arg: &str) -> String {
    use anvil_agent::memory::MemoryCategory;

    let memory_dir = agent.workspace().join(".anvil/memory");
    let store = MemoryStore::new(memory_dir);

    if arg.is_empty() {
        let entries = store.load_all();
        if entries.is_empty() {
            return "no stored patterns. use /memory add <pattern> to save one".to_string();
        }
        let mut output = format!("{} stored pattern(s):\n", entries.len());
        for entry in &entries {
            output.push_str(&format!(
                "\n  [{}] ({})\n  {}\n",
                entry.filename,
                entry.category.label(),
                entry.content
            ));
        }
        return output;
    }

    // /memory add [category:tag] <pattern>
    if let Some(rest) = arg.strip_prefix("add ") {
        let rest = rest.trim();
        if rest.is_empty() {
            return "usage: /memory add <pattern> or /memory add category:<tag> <pattern>"
                .to_string();
        }

        let (category, pattern) = if let Some(tagged) = rest.strip_prefix("category:") {
            let parts: Vec<&str> = tagged.splitn(2, ' ').collect();
            if parts.len() < 2 || parts[1].trim().is_empty() {
                return "usage: /memory add category:<tag> <pattern>".to_string();
            }
            (
                Some(MemoryCategory::from_tag(parts[0])),
                parts[1].trim().to_string(),
            )
        } else {
            (None, rest.to_string())
        };

        match store.add_with_category(&pattern, category.as_ref()) {
            Ok(filename) => {
                let cat_label = category.as_ref().map(|c| c.label()).unwrap_or("Note");
                format!("saved ({cat_label}): {filename}")
            }
            Err(e) => format!("failed to save: {e}"),
        }
    } else if arg == "clear" {
        match store.clear() {
            Ok(count) => format!("removed {count} pattern(s)"),
            Err(e) => format!("failed to clear: {e}"),
        }
    } else if let Some(query) = arg.strip_prefix("search ") {
        let query = query.trim();
        if query.is_empty() {
            return "usage: /memory search <keyword>".to_string();
        }
        let results = store.search(query);
        if results.is_empty() {
            format!("no memories matching '{query}'")
        } else {
            let mut output = format!("{} match(es) for '{query}':\n", results.len());
            for entry in &results {
                output.push_str(&format!(
                    "\n  [{}] ({})\n  {}\n",
                    entry.filename,
                    entry.category.label(),
                    entry.content
                ));
            }
            output
        }
    } else if let Some(filename) = arg.strip_prefix("rm ") {
        let filename = filename.trim();
        match store.remove(filename) {
            Ok(true) => format!("removed: {filename}"),
            Ok(false) => format!("not found: {filename}"),
            Err(e) => format!("failed to remove: {e}"),
        }
    } else {
        [
            "usage:",
            "  /memory                          list all",
            "  /memory add <pattern>            save a note",
            "  /memory add category:<tag> <pat>  save with category (convention|gotcha|pattern)",
            "  /memory search <keyword>         search by keyword",
            "  /memory rm <filename>            remove one entry",
            "  /memory clear                    remove all",
        ]
        .join("\n")
    }
}

/// Handle `/mcp` — show connected MCP servers and their tools.
async fn mcp_command(agent: &Agent, arg: &str) -> String {
    let mcp = agent.mcp();
    let status = mcp.server_status().await;

    if status.is_empty() && arg.is_empty() {
        return "no MCP servers configured. add servers in .anvil/config.toml under [mcp]"
            .to_string();
    }

    if arg.is_empty() {
        // List servers and tools
        let mut output = format!("{} MCP server(s):\n", status.len());
        for (name, tool_count, alive) in &status {
            let state = if *alive { "connected" } else { "disconnected" };
            output.push_str(&format!("\n  {name} ({state}, {tool_count} tools)"));
        }

        let tools = mcp.tools().await;
        if !tools.is_empty() {
            output.push_str("\n\ntools:");
            for tool in &tools {
                output.push_str(&format!(
                    "\n  {} — {}",
                    tool.qualified_name, tool.description
                ));
            }
        }

        return output;
    }

    if arg == "shutdown" {
        mcp.shutdown().await;
        return "all MCP servers shut down".to_string();
    }

    format!("unknown subcommand '{arg}'. usage: /mcp, /mcp shutdown")
}

/// Handle `/persona` — list, activate, or deactivate character personas.
///
/// When a persona is activated, auto-activates the `kids-first` skill
/// so the agent immediately behaves in a kid-friendly way. Mentions
/// the other kids skills (`kids-story`, `kids-game`) so the user knows
/// they exist.
fn persona_command(agent: &mut Agent, arg: &str) -> String {
    if arg.is_empty() {
        let personas = anvil_agent::builtin_personas();
        let active = agent.persona().map(|p| p.key.as_str());

        let mut output = String::from("character personas:\n");
        for p in &personas {
            let marker = if active == Some(&p.key) { " *" } else { "" };
            output.push_str(&format!("\n  {}{} — {}", p.key, marker, p.description));
        }
        if active.is_some() {
            output.push_str("\n\n  (* = active)");
        }
        output.push_str("\n\nusage: /persona <name> or /persona clear");
        return output;
    }

    if arg == "clear" {
        agent.set_persona(None);
        agent.clear_skills();
        return "persona and skills deactivated".to_string();
    }

    match anvil_agent::find_persona(arg) {
        Some(persona) => {
            let greeting = persona.greeting.clone();
            let name = persona.name.clone();
            agent.set_persona(Some(persona));

            // Auto-activate kids-first skill for immediate fun
            let mut skill_note = String::new();
            let loader = SkillLoader::new(agent.workspace());
            if let Ok(skill) = loader.get("kids-first") {
                agent.activate_skill(skill);
                skill_note = "\n\n  ready to make cool stuff! just say what you like.\
                     \n  try: /skill kids-story  (story mode)\
                     \n       /skill kids-game   (build a game)"
                    .to_string();
            }

            format!("{name} activated!\n\n{greeting}{skill_note}")
        }
        None => {
            let available: Vec<String> = anvil_agent::builtin_personas()
                .iter()
                .map(|p| p.key.clone())
                .collect();
            format!(
                "unknown persona '{}'. available: {}",
                arg,
                available.join(", ")
            )
        }
    }
}

fn think_command(agent: &mut Agent) -> String {
    let new_state = !agent.show_thinking();
    agent.set_show_thinking(new_state);
    if new_state {
        "thinking blocks: visible".to_string()
    } else {
        "thinking blocks: hidden".to_string()
    }
}

fn stats_text(agent: &Agent, usage: &TokenUsage) -> String {
    let u = usage;
    let msg_count = agent.messages().len();
    let cost_str = match u.estimated_cost_usd {
        Some(c) if c > 0.0 => format!("${c:.4}"),
        _ => "$0.00 (local)".to_string(),
    };
    let mut text = format!(
        "session:     {}\n\
         model:       {}\n\
         backend:     {}\n\
         messages:    {msg_count}\n\
         requests:    {}\n\
         tokens:      {} prompt + {} completion = {} total\n\
         cost:        {cost_str}",
        &agent.session_id()[..8],
        agent.model(),
        agent.backend(),
        u.request_count,
        u.prompt_tokens,
        u.completion_tokens,
        u.total_tokens,
    );

    // Show model routes if any are configured
    let routes = agent.router().routes();
    if !routes.is_empty() {
        let route_strs: Vec<String> = routes
            .iter()
            .map(|(tool, model)| format!("{tool} → {model}"))
            .collect();
        text.push_str(&format!("\nroutes:      {}", route_strs.join(", ")));
    }

    if agent.show_thinking() {
        text.push_str("\nthinking:    visible");
    }

    let extra = agent.extra_env();
    if !extra.is_empty() {
        text.push_str(&format!("\nenv pass:    {}", extra.join(", ")));
    }

    text
}

async fn model_command(agent: &mut Agent, arg: &str) -> String {
    let profiles = anvil_config::load_bundled_profiles();

    if arg.is_empty() {
        // Show current model info + numbered list of available models
        let mut info = format!("current: {} *\n", agent.model());

        // Show profile info for current model
        if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
            info.push_str(&format!("profile: {}", profile.name));
            if let Some(t) = profile.sampling.temperature {
                info.push_str(&format!(" (temp={t}"));
            }
            if let Some(p) = profile.sampling.top_p {
                info.push_str(&format!(", top_p={p}"));
            }
            if profile.context.default_window > 0 {
                info.push_str(&format!(", ctx={}", profile.context.default_window));
            }
            info.push(')');
        } else {
            info.push_str("profile: (none)");
        }

        // Discover and list available models
        let models = discover_models(agent).await;
        if !models.is_empty() {
            info.push_str("\n\navailable models:");
            for (i, model) in models.iter().enumerate() {
                let marker = if *model == agent.model() { " *" } else { "" };
                let profile_tag =
                    if let Some(p) = anvil_config::find_matching_profile(&profiles, model) {
                        format!("  ({})", p.name)
                    } else {
                        String::new()
                    };
                info.push_str(&format!("\n  {:>2}. {model}{marker}{profile_tag}", i + 1));
            }
            info.push_str("\n\nusage: /model <name> or /model <number>");
        }

        return info;
    }

    // Discover available models
    let models = discover_models(agent).await;

    // Check if arg is a number (picker selection)
    let model_name = if let Ok(num) = arg.parse::<usize>() {
        if !models.is_empty() && num >= 1 && num <= models.len() {
            models[num - 1].clone()
        } else if models.is_empty() {
            return "can't pick by number — backend unreachable. use /model <name>".to_string();
        } else {
            return format!("invalid number. pick 1-{}", models.len());
        }
    } else if models.is_empty() {
        // Can't discover — just set the model directly
        arg.to_string()
    } else if models
        .iter()
        .any(|m| m == arg || m.starts_with(&format!("{arg}:")))
    {
        if models.contains(&arg.to_string()) {
            arg.to_string()
        } else {
            models
                .iter()
                .find(|m| m.starts_with(&format!("{arg}:")))
                .cloned()
                .unwrap_or_else(|| arg.to_string())
        }
    } else {
        // Try fuzzy match — case-insensitive substring
        let arg_lower = arg.to_lowercase();
        if let Some(matched) = models
            .iter()
            .find(|m| m.to_lowercase().contains(&arg_lower))
        {
            matched.clone()
        } else {
            let available = models.join(", ");
            return format!("model '{arg}' not found. available: {available}");
        }
    };

    agent.set_model(model_name.clone());

    // Apply matching model profile
    let mut result = format!("switched to model: {model_name}");
    if let Some(profile) = anvil_config::find_matching_profile(&profiles, &model_name) {
        agent.apply_model_profile(profile);
        result.push_str(&format!(" (profile: {})", profile.name));
    } else {
        // Clear any previous profile's sampling overrides
        agent.clear_sampling();
        result.push_str(" (no profile)");
    }

    result
}

/// Discover available models from the backend.
///
/// Uses `/api/tags` for Ollama, `/v1/models` for other backends.
async fn discover_models(agent: &Agent) -> Vec<String> {
    let base = agent.base_url().trim_end_matches("/v1");

    match agent.backend() {
        anvil_config::BackendKind::Ollama => {
            let url = format!("{base}/api/tags");
            match reqwest::get(&url).await {
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(body) => body["models"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["name"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            }
        }
        _ => {
            // OpenAI-compatible /v1/models endpoint
            let url = format!("{}/models", agent.base_url().trim_end_matches('/'));
            match reqwest::get(&url).await {
                Ok(resp) => match resp.json::<serde_json::Value>().await {
                    Ok(body) => body["data"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m["id"].as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            }
        }
    }
}

/// Handle `/ralph` — parse arguments and start an interactive Ralph Loop.
///
/// # Usage
/// - `/ralph` — show help
/// - `/ralph fix tests --verify cargo test` — run loop with prompt and verify command
/// - `/ralph fix tests --verify cargo test --max-iterations 5` — with iteration limit
fn ralph_command(arg: &str) -> CommandResult {
    if arg.is_empty() {
        return CommandResult::Handled(
            [
                "Ralph Loop (autonomous mode):",
                "",
                "Usage:",
                "  /ralph <prompt> --verify <cmd>",
                "  /ralph <prompt> --verify <cmd> --max-iterations 5",
                "",
                "Example:",
                "  /ralph fix all failing tests --verify cargo test",
                "",
                "The agent runs your prompt, then checks the verify command.",
                "If verify fails, the failure output is fed back for another attempt.",
                "Ctrl+C stops the loop and returns to the prompt.",
            ]
            .join("\n"),
        );
    }

    // Parse --verify and --max-iterations from the argument string
    let (prompt, verify_cmd, max_iter) = match parse_ralph_args(arg) {
        Ok(parsed) => parsed,
        Err(e) => return CommandResult::Handled(format!("error: {e}")),
    };

    if verify_cmd.is_empty() {
        return CommandResult::Handled(
            "error: --verify <cmd> is required. Example: /ralph fix tests --verify cargo test"
                .to_string(),
        );
    }

    CommandResult::Ralph(RalphArgs {
        prompt,
        verify_command: verify_cmd,
        max_iterations: max_iter,
    })
}

/// Parse `/ralph` arguments: `<prompt> --verify <cmd> [--max-iterations N]`
fn parse_ralph_args(input: &str) -> Result<(String, String, usize), String> {
    let mut prompt_parts = Vec::new();
    let mut verify_parts = Vec::new();
    let mut max_iterations: usize = 10;
    let mut in_verify = false;
    let mut expect_max_iter = false;

    for token in input.split_whitespace() {
        if token == "--verify" {
            in_verify = true;
            continue;
        }
        if token == "--max-iterations" {
            expect_max_iter = true;
            continue;
        }
        if expect_max_iter {
            max_iterations = token
                .parse()
                .map_err(|_| format!("invalid --max-iterations value: {token}"))?;
            expect_max_iter = false;
            continue;
        }
        // Once we hit --verify, everything after goes to verify (until --max-iterations)
        if in_verify {
            verify_parts.push(token);
        } else {
            prompt_parts.push(token);
        }
    }

    let prompt = prompt_parts.join(" ");
    let verify_cmd = verify_parts.join(" ");

    Ok((prompt, verify_cmd, max_iterations))
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
fn backend_command(agent: &mut Agent, arg: &str) -> CommandResult {
    if arg.is_empty() {
        return CommandResult::Handled(format!(
            "backend: {}\nurl:     {}",
            agent.backend(),
            agent.base_url()
        ));
    }

    let parts: Vec<&str> = arg.splitn(3, ' ').collect();
    let subcommand = parts[0];

    // Handle start/stop subcommands
    if subcommand == "start" {
        let backend_type = parts.get(1).copied().unwrap_or("");
        let model_path = parts.get(2).copied().unwrap_or("");

        if backend_type.is_empty() || model_path.is_empty() {
            return CommandResult::Handled(
                "usage: /backend start llama <model_path.gguf> [--port 8080]".to_string(),
            );
        }

        // Parse optional port from model_path (could contain --port N)
        let (actual_model, port) = parse_backend_start_args(model_path);

        return CommandResult::BackendStart(BackendStartArgs {
            backend_type: backend_type.to_string(),
            model_path: actual_model,
            port,
        });
    }

    if subcommand == "stop" {
        return CommandResult::BackendStop;
    }

    // Original backend switching logic
    let backend_str = subcommand;
    let url = parts.get(1).map(|s| s.trim());

    let backend = match backend_str {
        "ollama" => anvil_config::BackendKind::Ollama,
        "llama" | "llama-server" => anvil_config::BackendKind::LlamaServer,
        "mlx" => anvil_config::BackendKind::Mlx,
        "custom" => anvil_config::BackendKind::Custom,
        _ => {
            return CommandResult::Handled(format!(
                "unknown backend '{}'. options: ollama, llama, mlx, custom, start, stop",
                backend_str
            ))
        }
    };

    let base_url = match url {
        Some(u) => u.to_string(),
        None => match backend {
            anvil_config::BackendKind::Ollama => "http://localhost:11434/v1".to_string(),
            anvil_config::BackendKind::LlamaServer => "http://localhost:8080/v1".to_string(),
            anvil_config::BackendKind::Mlx => "http://localhost:8080/v1".to_string(),
            anvil_config::BackendKind::Custom => {
                return CommandResult::Handled(
                    "custom backend requires a URL: /backend custom http://host:port/v1"
                        .to_string(),
                )
            }
        },
    };

    agent.set_backend(backend.clone(), base_url.clone());
    CommandResult::Handled(format!("switched to {} at {}", backend, base_url))
}

/// Parse model path and optional --port from backend start args.
fn parse_backend_start_args(input: &str) -> (String, u16) {
    let parts: Vec<&str> = input.split_whitespace().collect();
    let mut model_path = String::new();
    let mut port: u16 = 8080;
    let mut expect_port = false;

    for part in parts {
        if part == "--port" {
            expect_port = true;
            continue;
        }
        if expect_port {
            port = part.parse().unwrap_or(8080);
            expect_port = false;
            continue;
        }
        if model_path.is_empty() {
            model_path = part.to_string();
        }
    }

    (model_path, port)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ralph_args_basic() {
        let (prompt, verify, max) = parse_ralph_args("fix tests --verify cargo test").unwrap();
        assert_eq!(prompt, "fix tests");
        assert_eq!(verify, "cargo test");
        assert_eq!(max, 10);
    }

    #[test]
    fn parse_ralph_args_with_max_iterations() {
        let (prompt, verify, max) =
            parse_ralph_args("fix it --verify make check --max-iterations 5").unwrap();
        assert_eq!(prompt, "fix it");
        assert_eq!(verify, "make check");
        assert_eq!(max, 5);
    }

    #[test]
    fn parse_ralph_args_no_verify() {
        let (prompt, verify, _) = parse_ralph_args("fix tests").unwrap();
        assert_eq!(prompt, "fix tests");
        assert!(verify.is_empty());
    }

    #[test]
    fn parse_ralph_args_invalid_max() {
        let result = parse_ralph_args("fix --verify test --max-iterations abc");
        assert!(result.is_err());
    }

    #[test]
    fn ralph_command_no_args_shows_help() {
        let result = ralph_command("");
        assert!(matches!(result, CommandResult::Handled(_)));
    }

    #[test]
    fn ralph_command_missing_verify_shows_error() {
        let result = ralph_command("fix tests");
        match result {
            CommandResult::Handled(msg) => assert!(msg.contains("--verify")),
            _ => panic!("expected Handled with error"),
        }
    }

    #[test]
    fn ralph_command_valid_returns_ralph() {
        let result = ralph_command("fix tests --verify cargo test");
        match result {
            CommandResult::Ralph(args) => {
                assert_eq!(args.prompt, "fix tests");
                assert_eq!(args.verify_command, "cargo test");
                assert_eq!(args.max_iterations, 10);
            }
            _ => panic!("expected Ralph variant"),
        }
    }
}
