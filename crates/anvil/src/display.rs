//! Terminal display — banners, spinners, session summaries, formatting.
//!
//! Pure output functions with no state. The interactive loop calls these
//! to render UI elements. Follows the pi.dev/goose pattern of separating
//! display from orchestration.

use anvil_agent::Agent;
use anvil_llm::TokenUsage;
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use std::io::{self, Write};

use crate::render::Renderer;

// ── Spinner ──────────────────────────────────────────────────────────

/// Spawn a spinner task that shows elapsed time while waiting for LLM response.
///
/// Cancel the returned token to stop the spinner and clear the line.
pub fn spawn_spinner(cancel: tokio_util::sync::CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let start = std::time::Instant::now();
        let mut i = 0;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(80));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
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
    })
}

// ── Banner & Welcome ─────────────────────────────────────────────────

/// Print the startup banner with model info and persona theming.
pub fn print_banner(agent: &Agent) {
    if let Some(persona) = agent.persona() {
        let (border, icon) = match persona.key.as_str() {
            "sparkle" => ("✨", "🦄"),
            "bolt" => ("⚡", "🤖"),
            "codebeard" => ("⚓", "🏴\u{200d}☠\u{fe0f}"),
            _ => ("─", "🔨"),
        };
        println!(
            "{border}{border}{border} {icon} {} {border}{border}{border}",
            persona.name
        );
        println!();
        println!("  {}", persona.greeting);
        println!();

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
        let profiles = anvil_config::load_bundled_profiles();
        if let Some(profile) = anvil_config::find_matching_profile(&profiles, agent.model()) {
            if let Some(ref kv) = profile.kv_cache {
                println!(
                    "  kv cache: K={} V={} | ctx: {}",
                    kv.type_k, kv.type_v, kv.recommended_context
                );
            }
        }
        println!("  session: {}", &agent.session_id()[..8]);
        println!("  cwd:     {}", agent.workspace().display());
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
pub async fn print_model_hint(agent: &Agent) {
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

/// First-run welcome for new users.
pub fn print_first_run_welcome() {
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
    println!("│    /persona codebeard 🏴\u{200d}☠\u{fe0f}  Captain Codebeard      │");
    println!("│                                                  │");
    println!("│  Then just say what you like — cats, space,      │");
    println!("│  dragons — and watch something cool happen!      │");
    println!("│                                                  │");
    println!("│  Or just start typing to ask me anything.        │");
    println!("╰─────────────────────────────────────────────────╯");
    println!();
}

// ── Session Summary ──────────────────────────────────────────────────

/// Print a session summary on exit.
pub fn print_session_summary(
    duration: std::time::Duration,
    usage: &TokenUsage,
    tool_counts: &std::collections::HashMap<String, u32>,
    files_created: &[String],
    renderer: &dyn Renderer,
) {
    let mins = duration.as_secs() / 60;
    let secs = duration.as_secs() % 60;
    let duration_str = if mins > 0 {
        format!("{mins} min {secs}s")
    } else {
        format!("{secs}s")
    };

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
    println!("{}", renderer.session_summary_header());
    println!("│  Duration: {:<28}│", duration_str);
    println!(
        "│  Tokens:   {:<28}│",
        format_token_count(usage.total_tokens)
    );
    println!("│  Tools:    {:<28}│", tool_str);

    if !files_created.is_empty() {
        let file_list: Vec<&str> = files_created
            .iter()
            .map(|f| f.rsplit('/').next().unwrap_or(f))
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

    renderer.render_session_footer(files_created);
    println!("╰───────────────────────────────────────╯");
}

// ── Formatting Helpers ───────────────────────────────────────────────

/// Format token count for compact display (e.g., 1234 → "1.2k").
pub fn format_token_count(tokens: u64) -> String {
    if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

/// Map tool names to display icons.
pub fn tool_icon(name: &str) -> &'static str {
    match name {
        "shell" => "⚙",
        "file_read" => "📄",
        "file_write" => "📝",
        "file_edit" => "✏",
        "grep" => "🔍",
        "find" => "🔍",
        "ls" => "📂",
        "git_status" | "git_diff" | "git_log" | "git_commit" => "📊",
        _ => "🔧",
    }
}

/// Truncate a string for display, collapsing newlines and adding `[...]`.
pub fn truncate_display(s: &str, max: usize) -> String {
    let oneline = s.replace('\n', " ").replace('\r', "");
    if oneline.len() <= max {
        oneline
    } else {
        let boundary = oneline
            .char_indices()
            .take_while(|(i, _)| *i <= max)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        format!("{}[...]", &oneline[..boundary])
    }
}

/// Format an RFC3339 timestamp as a human-readable "time ago" string.
pub fn format_time_ago(timestamp: &str) -> String {
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
