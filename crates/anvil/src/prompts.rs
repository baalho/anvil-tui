//! User input and permission prompting.
//!
//! Separated from the interactive loop so the orchestrator doesn't
//! mix I/O mechanics with control flow.

use anvil_tools::PermissionDecision;
use anyhow::Result;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::{execute, terminal};
use std::io::{self, BufRead, Write};

/// Read a line of input from stdin, supporting backslash continuation.
///
/// Returns `None` on EOF. Lines ending with `\` are joined with the
/// next line, showing a `...` continuation prompt.
pub fn read_input(stdin: &io::Stdin) -> Option<String> {
    let mut full_input = String::new();
    let reader = stdin.lock();

    for line in reader.lines() {
        match line {
            Ok(line) => {
                if line.ends_with('\\') {
                    full_input.push_str(&line[..line.len() - 1]);
                    full_input.push('\n');
                    let _ = execute!(
                        io::stdout(),
                        SetForegroundColor(Color::Green),
                        Print("...  "),
                        ResetColor,
                    );
                    let _ = io::stdout().flush();
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

/// Prompt the user for a tool permission decision using single-keypress input.
///
/// Shows `Allow <tool>(<args>)? [y/n/a]` and reads one key:
/// - `y`/Enter → Allow
/// - `n` → Deny
/// - `a` → AllowAlways
pub fn prompt_permission(tool_name: &str, arguments: &str) -> Result<PermissionDecision> {
    let short_args = crate::display::truncate_display(arguments, 60);
    execute!(
        io::stdout(),
        SetForegroundColor(Color::Yellow),
        Print(format!("  Allow {tool_name}({short_args})? [y/n/a] ")),
        ResetColor,
    )?;
    io::stdout().flush()?;

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
