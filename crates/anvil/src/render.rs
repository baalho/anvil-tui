//! Output rendering pipeline — abstracts how agent output reaches the user.
//!
//! The [`Renderer`] trait decouples the interactive loop from display logic.
//! [`TerminalRenderer`] handles standard output. [`KidsRenderer`] wraps it
//! to show fun messages instead of JSON schemas and shell metadata.
//!
//! [`select_renderer`] picks the correct implementation based on kids mode.
//! The interactive loop's `TurnPolicy` holds the renderer for the current
//! turn — the loop calls trait methods uniformly without knowing which
//! implementation is active.

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

    /// Render a tool execution result with the raw output text.
    fn render_tool_result(&self, tool_name: &str, icon: &str, result_text: &str);

    /// Render a command result (slash command output).
    fn render_command_result(&self, text: &str);

    /// Render an error message.
    fn render_error(&self, message: &str);

    /// Render a status/info message (non-content, e.g., "retrying...").
    fn render_info(&self, message: &str);

    /// Format the status line shown in the readline prompt.
    /// Returns empty string to hide the status line entirely.
    fn format_status(&self, mode: &str, model: &str, persona: Option<&str>) -> String;

    /// Format the session summary header.
    fn session_summary_header(&self) -> &str;

    /// Render optional session summary footer (e.g., kids "cool things" count).
    fn render_session_footer(&self, _files_created: &[String]) {}
}

/// Terminal renderer — renders agent output inline using crossterm.
///
/// This is the default renderer for standard interactive mode. Uses ANSI
/// colors for tool results, errors, and status messages.
#[derive(Default)]
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

    fn render_tool_result(&self, tool_name: &str, icon: &str, result_text: &str) {
        let lines = result_text.lines().count();
        let chars = result_text.len();
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print(format!(
                "  {icon} {tool_name}: {lines} lines, {chars} chars\n"
            )),
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

    fn format_status(&self, mode: &str, model: &str, persona: Option<&str>) -> String {
        if let Some(persona) = persona {
            format!("[{}|{}|{}]", persona, mode, model)
        } else {
            format!("[{}|{}]", mode, model)
        }
    }

    fn session_summary_header(&self) -> &str {
        "╭─ Session Summary ─────────────────────╮"
    }
}

/// Kids renderer — wraps `TerminalRenderer` with child-friendly output.
///
/// Replaces tool schemas and shell metadata with fun messages. Hides
/// technical details (exit codes, file paths, JSON) that would confuse
/// a 7-year-old. The interactive loop doesn't know this renderer exists —
/// it calls the same `Renderer` trait methods uniformly.
pub struct KidsRenderer {
    inner: TerminalRenderer,
}

impl Default for KidsRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl KidsRenderer {
    pub fn new() -> Self {
        Self {
            inner: TerminalRenderer::new(),
        }
    }
}

impl Renderer for KidsRenderer {
    fn render_content_delta(&self, text: &str) {
        self.inner.render_content_delta(text);
    }

    fn render_thinking_delta(&self, text: &str) {
        self.inner.render_thinking_delta(text);
    }

    fn render_thinking_start(&self) {
        self.inner.render_thinking_start();
    }

    fn render_thinking_end(&self) {
        self.inner.render_thinking_end();
    }

    fn render_tool_pending(&self, tool_name: &str, _icon: &str, _args_preview: &str) {
        let msg = match tool_name {
            "file_write" => "  ✨ *writing some magic code...*\n",
            "file_edit" => "  ✨ *changing the magic...*\n",
            "shell" => "  🚀 *running it...*\n",
            _ => return, // hide other tools entirely
        };
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::Magenta),
            Print(msg),
            ResetColor,
        );
    }

    fn render_tool_result(&self, tool_name: &str, _icon: &str, result_text: &str) {
        if tool_name == "shell" {
            // Strip shell metadata — kids see only program output
            let clean: String = result_text
                .lines()
                .map(|l| l.trim())
                .filter(|line| {
                    !line.starts_with("exit code:")
                        && !line.starts_with("stdout:")
                        && !line.starts_with("stderr:")
                        && !line.starts_with("error: command timed out")
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !clean.trim().is_empty() {
                let _ = crossterm::execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Green),
                    Print(format!("{}\n", clean.trim())),
                    ResetColor,
                );
            }
            if result_text.contains("timed out") {
                let _ = crossterm::execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Yellow),
                    Print("  ⏰ *oops, that took too long! let me try again...*\n"),
                    ResetColor,
                );
            }
        }
        // file_write/file_edit results are silently swallowed
    }

    fn render_command_result(&self, text: &str) {
        self.inner.render_command_result(text);
    }

    fn render_error(&self, message: &str) {
        // Show kid-friendly error instead of raw message
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::Yellow),
            Print("  🤔 *hmm, something went wonky! let me try again...*\n"),
            ResetColor,
        );
        let _ = message; // suppress unused warning
    }

    fn render_info(&self, message: &str) {
        self.inner.render_info(message);
    }

    fn format_status(&self, _mode: &str, _model: &str, _persona: Option<&str>) -> String {
        // Kids don't need to see [coding|qwen3-coder:30b]
        String::new()
    }

    fn session_summary_header(&self) -> &str {
        "╭─ ✨ What You Made! ✨ ──────────────╮"
    }

    fn render_session_footer(&self, files_created: &[String]) {
        let count = files_created.len();
        if count > 0 {
            println!("│                                       │");
            println!(
                "│  ✨ You made {} cool thing{}! ✨        │",
                count,
                if count == 1 { "" } else { "s" }
            );
        }
    }
}

/// Create the appropriate renderer based on whether kids mode is active.
pub fn select_renderer(is_kids: bool) -> Box<dyn Renderer> {
    if is_kids {
        Box::new(KidsRenderer::new())
    } else {
        Box::new(TerminalRenderer::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_renderer_format_status_with_persona() {
        let r = TerminalRenderer::new();
        assert_eq!(
            r.format_status("coding", "qwen3", Some("sparkle")),
            "[sparkle|coding|qwen3]"
        );
    }

    #[test]
    fn terminal_renderer_format_status_without_persona() {
        let r = TerminalRenderer::new();
        assert_eq!(r.format_status("coding", "qwen3", None), "[coding|qwen3]");
    }

    #[test]
    fn kids_renderer_hides_status() {
        let r = KidsRenderer::new();
        assert_eq!(r.format_status("coding", "qwen3", Some("sparkle")), "");
    }

    #[test]
    fn kids_renderer_session_header() {
        let r = KidsRenderer::new();
        assert!(r.session_summary_header().contains("What You Made"));
    }

    #[test]
    fn select_renderer_returns_correct_type() {
        let normal = select_renderer(false);
        assert_eq!(
            normal.format_status("coding", "qwen3", None),
            "[coding|qwen3]"
        );

        let kids = select_renderer(true);
        assert_eq!(kids.format_status("coding", "qwen3", Some("sparkle")), "");
    }
}
