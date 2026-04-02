//! Output rendering pipeline — abstracts how agent output reaches the user.
//!
//! # Why this exists
//! All agent output currently goes through `println!` and crossterm in
//! `interactive.rs`. This works for text but has no path for images, SVG,
//! tables, or other rich content. The `Renderer` trait provides a seam:
//! - v1.5: `TerminalRenderer` — extracts current inline rendering
//! - Future: `ImageRenderer` (Kitty/iTerm2/Sixel), `WebRenderer`, etc.
//!
//! # How it works
//! The interactive loop calls `Renderer` methods instead of writing directly
//! to stdout. Each renderer implementation decides how to display content
//! based on the output type and terminal capabilities.

use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use std::io::{self, Write};

/// Renders agent output to the user. Implementations handle different
/// output targets (terminal, web, etc.) and content types (text, images).
pub trait Renderer {
    /// Render a text delta from the assistant's streaming response.
    fn render_content_delta(&self, text: &str);

    /// Render a thinking block delta (chain-of-thought).
    fn render_thinking_delta(&self, text: &str);

    /// Render the start of a thinking block (box-drawing open).
    fn render_thinking_start(&self);

    /// Render the end of a thinking block (box-drawing close).
    fn render_thinking_end(&self);

    /// Render a tool call that's about to execute.
    fn render_tool_pending(&self, tool_name: &str, icon: &str, args_preview: &str);

    /// Render a tool execution result.
    fn render_tool_result(&self, tool_name: &str, icon: &str, lines: usize, chars: usize);

    /// Render a command result (slash command output).
    fn render_command_result(&self, text: &str);

    /// Render an error message.
    fn render_error(&self, message: &str);

    /// Render a status/info message (non-content, e.g., "retrying...").
    fn render_info(&self, message: &str);
}

/// Terminal renderer — renders agent output inline using crossterm.
///
/// This is the default renderer. It handles text content via direct stdout
/// writes and uses ANSI colors for tool results, errors, and status messages.
/// Future content types (images, SVG) will need a different renderer or
/// terminal protocol support (Kitty, iTerm2, Sixel).
pub struct TerminalRenderer;

impl TerminalRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Renderer for TerminalRenderer {
    fn render_content_delta(&self, text: &str) {
        print!("{text}");
        let _ = io::stdout().flush();
    }

    fn render_thinking_delta(&self, text: &str) {
        // Prefix each newline with box-drawing continuation
        let prefixed = text.replace('\n', "\n  │ ");
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print(&prefixed),
            ResetColor,
        );
        let _ = io::stdout().flush();
    }

    fn render_thinking_start(&self) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print("  ╭─ thinking\n  │ "),
            ResetColor,
        );
    }

    fn render_thinking_end(&self) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print("\n  ╰─\n"),
            ResetColor,
        );
    }

    fn render_tool_pending(&self, tool_name: &str, icon: &str, args_preview: &str) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::Cyan),
            Print(format!("  {icon} {tool_name}")),
            SetForegroundColor(Color::DarkGrey),
            Print(format!(" ─ {args_preview}\n")),
            ResetColor,
        );
    }

    fn render_tool_result(&self, tool_name: &str, icon: &str, lines: usize, chars: usize) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("  {icon} {tool_name}: {lines} lines, {chars} chars\n")),
            ResetColor,
        );
    }

    fn render_command_result(&self, text: &str) {
        println!("{text}");
    }

    fn render_error(&self, message: &str) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::Red),
            Print(format!("error: {message}\n")),
            ResetColor,
        );
    }

    fn render_info(&self, message: &str) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("{message}\n")),
            ResetColor,
        );
    }
}

/// Truncate a string for display, adding "..." if truncated.
fn truncate_for_display(s: &str, max: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.len() > max {
        format!("{}...", &first_line[..max])
    } else if s.lines().count() > 1 {
        format!("{first_line}...")
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_for_display("hello", 100), "hello");
    }

    #[test]
    fn truncate_long_string_adds_ellipsis() {
        let long = "a".repeat(300);
        let result = truncate_for_display(&long, 200);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 203); // 200 + "..."
    }

    #[test]
    fn truncate_multiline_takes_first_line() {
        let multi = "first line\nsecond line\nthird line";
        let result = truncate_for_display(multi, 200);
        assert_eq!(result, "first line...");
    }
}
