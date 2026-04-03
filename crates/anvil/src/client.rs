//! IPC client — connects to the daemon and streams responses.
//!
//! Used by `anvil send`, `anvil daemon stop`, and `anvil daemon status`.
//! Stdout receives clean content (pipe-friendly). Stderr receives
//! diagnostics (tool calls, errors, thinking blocks).

use crate::ipc::{self, Request, Response};
use anyhow::{bail, Result};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use std::io::{self, Write};
use std::path::Path;
use tokio::net::UnixStream;

/// Connect to the daemon socket for the given workspace.
async fn connect(workspace: &Path) -> Result<UnixStream> {
    let path = ipc::socket_path(workspace);
    match UnixStream::connect(&path).await {
        Ok(stream) => Ok(stream),
        Err(e) => bail!(
            "cannot connect to daemon at {}: {e}\n\
             Is the daemon running? Start it with: anvil daemon start",
            path.display()
        ),
    }
}

/// Send a prompt to the daemon and stream the response to stdout/stderr.
///
/// Content deltas go to stdout (clean, pipe-friendly).
/// Tool calls, thinking, and errors go to stderr (diagnostics).
/// Returns exit code: 0 on success, 1 on error.
pub async fn send_prompt(workspace: &Path, text: &str, auto_approve: bool) -> Result<i32> {
    let stream = connect(workspace).await?;
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    // Send the prompt
    let request = Request::Prompt {
        text: text.to_string(),
        auto_approve,
    };
    ipc::write_frame(&mut writer, &request).await?;

    // Stream responses
    let mut exit_code = 0;
    loop {
        let response: Response = match ipc::read_frame(&mut reader).await? {
            Some(r) => r,
            None => break, // Daemon closed connection
        };

        match response {
            Response::Delta { text } => {
                print!("{text}");
                io::stdout().flush()?;
            }
            Response::Thinking { text } => {
                let _ = crossterm::execute!(
                    io::stderr(),
                    SetForegroundColor(Color::DarkGrey),
                    Print(&text),
                    ResetColor,
                );
            }
            Response::ToolPending { name, arguments } => {
                let short = if arguments.len() > 60 {
                    format!("{}...", &arguments[..60])
                } else {
                    arguments
                };
                let _ = crossterm::execute!(
                    io::stderr(),
                    SetForegroundColor(Color::Cyan),
                    Print(format!("  ⚙ {name}")),
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!(" — {short}\n")),
                    ResetColor,
                );
            }
            Response::ToolResult { name, lines, chars } => {
                let _ = crossterm::execute!(
                    io::stderr(),
                    SetForegroundColor(Color::DarkGrey),
                    Print(format!("  ✓ {name}: {lines} lines, {chars} chars\n")),
                    ResetColor,
                );
            }
            Response::TurnComplete => {
                println!();
                break;
            }
            Response::Error { message } => {
                let _ = crossterm::execute!(
                    io::stderr(),
                    SetForegroundColor(Color::Red),
                    Print(format!("error: {message}\n")),
                    ResetColor,
                );
                exit_code = 1;
                break;
            }
            // Status/Acknowledged are not expected in prompt flow
            _ => {}
        }
    }

    Ok(exit_code)
}

/// Query daemon status and print it.
pub async fn daemon_status(workspace: &Path) -> Result<()> {
    let stream = connect(workspace).await?;
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    ipc::write_frame(&mut writer, &Request::Status).await?;

    let response: Response = match ipc::read_frame(&mut reader).await? {
        Some(r) => r,
        None => bail!("daemon closed connection unexpectedly"),
    };

    match response {
        Response::StatusInfo {
            session_id,
            model,
            mode,
            uptime_secs,
            pid,
        } => {
            let hours = uptime_secs / 3600;
            let mins = (uptime_secs % 3600) / 60;
            let secs = uptime_secs % 60;

            println!("anvil daemon is running");
            println!("  pid:     {pid}");
            println!("  session: {}", &session_id[..8.min(session_id.len())]);
            println!("  model:   {model}");
            println!("  mode:    {mode}");
            println!("  uptime:  {hours}h {mins}m {secs}s");
            println!("  socket:  {}", ipc::socket_path(workspace).display());
        }
        Response::Error { message } => {
            bail!("daemon error: {message}");
        }
        _ => {
            bail!("unexpected response from daemon");
        }
    }

    Ok(())
}

/// Send a shutdown request to the daemon.
pub async fn daemon_stop(workspace: &Path) -> Result<()> {
    let stream = connect(workspace).await?;
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    ipc::write_frame(&mut writer, &Request::Shutdown).await?;

    let response: Response = match ipc::read_frame(&mut reader).await? {
        Some(r) => r,
        None => {
            println!("daemon stopped");
            return Ok(());
        }
    };

    match response {
        Response::Acknowledged => {
            println!("daemon stopping...");
        }
        Response::Error { message } => {
            bail!("daemon error: {message}");
        }
        _ => {
            println!("shutdown request sent");
        }
    }

    Ok(())
}
