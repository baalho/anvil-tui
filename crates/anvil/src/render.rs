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

    /// Render a structured tool result. Default falls back to `render_tool_result`.
    /// Override to handle `content_type` hints like "table" or "image".
    fn render_tool_output(&self, tool_name: &str, icon: &str, output: &anvil_tools::ToolOutput) {
        self.render_tool_result(tool_name, icon, output.text());
    }

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

    /// Render an image inline in the terminal (Kitty graphics protocol).
    /// Default: display the file path. Override for protocol-specific rendering.
    fn render_image(&self, path: &std::path::Path) {
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("  [image: {}]\n", path.display())),
            ResetColor,
        );
    }
}

/// Detected terminal capabilities — image protocol, Zellij, etc.
/// Probed once at startup and passed to the renderer.
#[derive(Debug, Clone, Default)]
pub struct TerminalCapabilities {
    /// Kitty graphics protocol supported (Kitty, WezTerm, iTerm2).
    pub kitty_graphics: bool,
    /// Running inside a Zellij session.
    pub zellij: bool,
    /// Zellij session name (if inside Zellij).
    pub zellij_session: Option<String>,
}

impl TerminalCapabilities {
    /// Detect capabilities from environment variables.
    pub fn detect() -> Self {
        let kitty_graphics = std::env::var("KITTY_WINDOW_ID").is_ok()
            || std::env::var("TERM_PROGRAM")
                .map(|v| {
                    let lower = v.to_lowercase();
                    lower.contains("wezterm") || lower.contains("iterm") || lower.contains("kitty")
                })
                .unwrap_or(false);

        let zellij = std::env::var("ZELLIJ").is_ok();
        let zellij_session = std::env::var("ZELLIJ_SESSION_NAME").ok();

        Self {
            kitty_graphics,
            zellij,
            zellij_session,
        }
    }
}

/// Terminal renderer — renders agent output inline using crossterm.
///
/// This is the default renderer for standard interactive mode. Uses ANSI
/// colors for tool results, errors, and status messages.
///
/// # `let _ =` pattern
/// Terminal write operations (`crossterm::execute!`, `write!`, `flush`) use
/// `let _ =` throughout this module. Terminal output failures are unrecoverable
/// — if stdout is broken, there's nothing useful to do with the error.
#[derive(Default)]
pub struct TerminalRenderer {
    capabilities: TerminalCapabilities,
    /// Workspace path for context file overflow.
    workspace: Option<std::path::PathBuf>,
}

impl TerminalRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a renderer with detected terminal capabilities.
    pub fn with_capabilities(capabilities: TerminalCapabilities) -> Self {
        Self {
            capabilities,
            workspace: None,
        }
    }

    /// Set the workspace path for context file overflow.
    pub fn set_workspace(&mut self, workspace: std::path::PathBuf) {
        self.workspace = Some(workspace);
    }

    /// Get the terminal capabilities.
    pub fn capabilities(&self) -> &TerminalCapabilities {
        &self.capabilities
    }

    /// Render a JSON array as an aligned table with box-drawing borders.
    /// Falls back to plain text if the data isn't a suitable array.
    fn render_table(&self, tool_name: &str, icon: &str, data: &serde_json::Value) {
        let rows = match data.as_array() {
            Some(arr) if !arr.is_empty() => arr,
            _ => {
                self.render_tool_result(tool_name, icon, &data.to_string());
                return;
            }
        };

        // Extract column names from the first row
        let columns: Vec<String> = match rows[0].as_object() {
            Some(obj) => obj.keys().cloned().collect(),
            None => {
                // Array of non-objects — render as single-column
                let text: String = rows
                    .iter()
                    .map(|v| v.as_str().unwrap_or(&v.to_string()).to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                self.render_tool_result(tool_name, icon, &text);
                return;
            }
        };

        // Calculate column widths (header vs data)
        let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
        for row in rows {
            for (i, col) in columns.iter().enumerate() {
                let val = row
                    .get(col)
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.len(),
                        other => other.to_string().len(),
                    })
                    .unwrap_or(0);
                if val > widths[i] {
                    widths[i] = val;
                }
            }
        }

        // Clamp to terminal width
        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(120);
        let total: usize = widths.iter().sum::<usize>() + (columns.len() * 3) + 1;
        if total > term_width {
            // Shrink the widest column to fit
            if let Some(max_idx) = widths
                .iter()
                .enumerate()
                .max_by_key(|(_, w)| *w)
                .map(|(i, _)| i)
            {
                let overflow = total - term_width;
                if widths[max_idx] > overflow + 3 {
                    widths[max_idx] -= overflow;
                }
            }
        }

        // Build table
        let mut out = String::new();
        // Header
        out.push_str("  ┌");
        for (i, w) in widths.iter().enumerate() {
            out.push_str(&"─".repeat(w + 2));
            out.push(if i < widths.len() - 1 { '┬' } else { '┐' });
        }
        out.push('\n');

        // Column names
        out.push_str("  │");
        for (i, col) in columns.iter().enumerate() {
            out.push_str(&format!(" {:<width$} │", col, width = widths[i]));
        }
        out.push('\n');

        // Separator
        out.push_str("  ├");
        for (i, w) in widths.iter().enumerate() {
            out.push_str(&"─".repeat(w + 2));
            out.push(if i < widths.len() - 1 { '┼' } else { '┤' });
        }
        out.push('\n');

        // Data rows
        for row in rows {
            out.push_str("  │");
            for (i, col) in columns.iter().enumerate() {
                let val = row
                    .get(col)
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                let truncated = if val.len() > widths[i] {
                    format!("{}…", &val[..widths[i] - 1])
                } else {
                    val
                };
                out.push_str(&format!(" {:<width$} │", truncated, width = widths[i]));
            }
            out.push('\n');
        }

        // Footer
        out.push_str("  └");
        for (i, w) in widths.iter().enumerate() {
            out.push_str(&"─".repeat(w + 2));
            out.push(if i < widths.len() - 1 { '┴' } else { '┘' });
        }
        out.push('\n');

        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::DarkGrey),
            Print(format!("  {icon} {tool_name}:\n")),
            ResetColor,
            Print(out),
        );
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

    fn render_tool_output(&self, tool_name: &str, icon: &str, output: &anvil_tools::ToolOutput) {
        let text = output.text();

        // Save overflow output to context file if it's long
        if let Some(workspace) = self.workspace.as_ref() {
            if let Ok(Some(path)) = crate::context::save_overflow(workspace, tool_name, text) {
                let _ = crossterm::execute!(
                    std::io::stdout(),
                    crossterm::style::SetForegroundColor(crossterm::style::Color::DarkGrey),
                    crossterm::style::Print(format!("  [full output: {}]\n", path.display())),
                    crossterm::style::ResetColor,
                );
            }
        }

        match output {
            anvil_tools::ToolOutput::Structured {
                content_type, data, ..
            } if content_type == "table" => {
                self.render_table(tool_name, icon, data);
            }
            _ => self.render_tool_result(tool_name, icon, text),
        }
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

    fn render_image(&self, path: &std::path::Path) {
        if !self.capabilities.kitty_graphics {
            // Fallback: just show the path
            let _ = crossterm::execute!(
                io::stdout(),
                SetForegroundColor(Color::DarkGrey),
                Print(format!("  [image: {}]\n", path.display())),
                ResetColor,
            );
            return;
        }

        // Kitty graphics protocol: read file, base64-encode, send escape sequence
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                let _ = crossterm::execute!(
                    io::stdout(),
                    SetForegroundColor(Color::Red),
                    Print(format!("  [image error: {e}]\n")),
                    ResetColor,
                );
                return;
            }
        };

        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);

        // Kitty protocol requires chunked transfer for large payloads.
        // Each chunk is max 4096 bytes of base64 data.
        let chunks: Vec<&str> = encoded
            .as_bytes()
            .chunks(4096)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect();

        let mut stdout = io::stdout();
        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i == chunks.len() - 1;
            if i == 0 {
                // First chunk: include format and action
                let _ = write!(
                    stdout,
                    "\x1b_Gf=100,a=T,t=d,m={};{}\x1b\\",
                    if is_last { 0 } else { 1 },
                    chunk
                );
            } else {
                // Continuation chunks
                let _ = write!(
                    stdout,
                    "\x1b_Gm={};{}\x1b\\",
                    if is_last { 0 } else { 1 },
                    chunk
                );
            }
        }
        let _ = writeln!(stdout);
        let _ = stdout.flush();
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
    /// Track content length to limit verbose responses.
    content_chars: std::sync::atomic::AtomicUsize,
}

impl Default for KidsRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl KidsRenderer {
    /// Max characters of LLM content to show kids (prevents walls of text).
    const MAX_CONTENT_CHARS: usize = 2000;

    pub fn new() -> Self {
        Self {
            inner: TerminalRenderer::new(),
            content_chars: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Create a kids renderer with detected terminal capabilities.
    pub fn with_capabilities(capabilities: TerminalCapabilities) -> Self {
        Self {
            inner: TerminalRenderer::with_capabilities(capabilities),
            content_chars: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl Renderer for KidsRenderer {
    fn render_content_delta(&self, text: &str) {
        use std::sync::atomic::Ordering;
        let current = self.content_chars.fetch_add(text.len(), Ordering::Relaxed);
        if current > Self::MAX_CONTENT_CHARS {
            // Already past limit — suppress
            return;
        }
        if current + text.len() > Self::MAX_CONTENT_CHARS {
            // This chunk crosses the limit — show truncation marker
            let remaining = Self::MAX_CONTENT_CHARS - current;
            if remaining > 0 {
                self.inner.render_content_delta(&text[..remaining]);
            }
            let _ = crossterm::execute!(
                io::stdout(),
                SetForegroundColor(Color::DarkGrey),
                Print("\n  [...]\n"),
                ResetColor,
            );
            return;
        }
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
            // Strip shell metadata, technical jargon, and file paths
            let clean: String = result_text
                .lines()
                .map(|l| l.trim())
                .filter(|line| {
                    !line.starts_with("exit code:")
                        && !line.starts_with("stdout:")
                        && !line.starts_with("stderr:")
                        && !line.starts_with("error: command timed out")
                        && !line.starts_with("Traceback")
                        && !line.contains("SyntaxWarning")
                        && !line.contains("DeprecationWarning")
                        && !line.starts_with("  File \"")
                })
                .take(30) // Limit output to 30 lines for kids
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
        // Map technical errors to kid-friendly messages
        let friendly = if message.contains("special characters") {
            "  ✨ *oops, let me try a different way!*\n"
        } else if message.contains("kids mode") || message.contains("isn't available") {
            "  ✨ *hmm, I can't do that one — let me try something else!*\n"
        } else if message.contains("needs a script file") {
            "  ✨ *let me write that to a file first!*\n"
        } else if message.contains("timed out") {
            "  ⏰ *that took too long! let me try again...*\n"
        } else if message.contains("connection") || message.contains("refused") {
            "  🤔 *my brain isn't responding — is the AI running?*\n"
        } else {
            "  🤔 *hmm, something went wonky! let me try again...*\n"
        };
        let _ = crossterm::execute!(
            io::stdout(),
            SetForegroundColor(Color::Yellow),
            Print(friendly),
            ResetColor,
        );
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

    fn render_image(&self, path: &std::path::Path) {
        self.inner.render_image(path);
    }
}

/// Create the appropriate renderer based on whether kids mode is active.
/// Select the appropriate renderer based on kids mode.
/// Detects terminal capabilities (Kitty graphics, Zellij) automatically.
pub fn select_renderer(is_kids: bool) -> Box<dyn Renderer> {
    let caps = TerminalCapabilities::detect();
    if is_kids {
        Box::new(KidsRenderer::with_capabilities(caps))
    } else {
        Box::new(TerminalRenderer::with_capabilities(caps))
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

    #[test]
    fn render_tool_output_default_falls_back_to_text() {
        // The default trait implementation calls render_tool_result
        let r = TerminalRenderer::new();
        let output = anvil_tools::ToolOutput::Text("hello".into());
        // Just verify it doesn't panic — output goes to stdout
        r.render_tool_output("test", "T", &output);
    }

    #[test]
    fn render_tool_output_structured_table() {
        let r = TerminalRenderer::new();
        let data = serde_json::json!([
            {"name": "src/", "type": "dir", "size": ""},
            {"name": "main.rs", "type": "file", "size": "1.2K"},
        ]);
        let output = anvil_tools::ToolOutput::Structured {
            text: "dir src/\nfile main.rs (1.2K)".into(),
            data,
            content_type: "table".into(),
        };
        // Verify it doesn't panic — table output goes to stdout
        r.render_tool_output("ls", "D", &output);
    }

    #[test]
    fn capabilities_default_is_all_false() {
        let caps = TerminalCapabilities::default();
        assert!(!caps.kitty_graphics);
        assert!(!caps.zellij);
        assert!(caps.zellij_session.is_none());
    }

    #[test]
    fn kitty_escape_sequence_format() {
        // Verify the Kitty protocol escape sequence format
        use base64::Engine;
        let data = b"PNG_DATA";
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        // Single chunk (< 4096 bytes)
        let expected = format!("\x1b_Gf=100,a=T,t=d,m=0;{}\x1b\\", encoded);
        assert!(expected.starts_with("\x1b_G"));
        assert!(expected.ends_with("\x1b\\"));
        assert!(expected.contains(&encoded));
    }

    #[test]
    fn render_image_fallback_no_panic() {
        // Without kitty support, render_image should just print the path
        let r = TerminalRenderer::new();
        let path = std::path::Path::new("/tmp/test.png");
        r.render_image(path);
    }
}
