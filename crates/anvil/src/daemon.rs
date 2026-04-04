//! Daemon server — accepts IPC connections and dispatches to the agent.
//!
//! # Architecture
//! Three concurrent tasks communicate through a single `mpsc` channel:
//!
//! 1. **Accept loop** — listens on the UDS, spawns per-connection handlers
//!    that read requests and enqueue `DaemonTask`s.
//! 2. **Signal handler** — catches SIGINT/SIGTERM, enqueues `DaemonTask::Shutdown`.
//! 3. **Dispatch loop** — the sole consumer, owns `&mut Agent`, processes
//!    tasks sequentially. Long turns block subsequent tasks (they queue).
//!
//! This design avoids `Arc<Mutex<Agent>>` entirely. The agent is never
//! accessed concurrently.
//!
//! # v2.0 upgrade from v1.9
//! The file watcher from `anvil watch` can optionally feed into the same
//! task queue, giving the daemon both IPC and filesystem triggers.

use crate::ipc::{self, Request, Response};
use anvil_agent::{dispatch_event, Agent, AgentEvent, Event};
use anvil_tools::PermissionDecision;
use anyhow::{bail, Result};
use std::time::{Duration, Instant};

/// Maximum time to wait for a single IPC frame write before dropping
/// the connection. Prevents a slow/suspended client from blocking the
/// dispatch loop via channel backpressure.
const WRITE_TIMEOUT: Duration = Duration::from_secs(3);
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Internal task queued for the dispatch loop.
/// Not exposed outside this module — clients see Request/Response.
enum DaemonTask {
    /// Run an agent turn with the given prompt.
    Prompt {
        text: String,
        reply_tx: mpsc::Sender<AgentEvent>,
        auto_approve: bool,
    },
    /// Query daemon status.
    Status { reply_tx: oneshot::Sender<Response> },
    /// Graceful shutdown.
    Shutdown,
}

/// Run the daemon server. Blocks until shutdown.
///
/// Binds a UDS listener, writes a PID file, and enters the dispatch loop.
/// On exit (signal or IPC shutdown), cleans up the socket and PID file.
/// The socket is workspace-scoped so multiple daemons can run concurrently.
pub async fn run_daemon(mut agent: Agent) -> Result<()> {
    let workspace = agent.workspace().to_path_buf();
    let sock_path = ipc::socket_path(&workspace);
    let pid_path = ipc::pid_path(&workspace);

    // Ensure socket directory exists with correct permissions
    ipc::ensure_socket_dir(&workspace)?;

    // Remove stale socket from a previous unclean shutdown
    if sock_path.exists() {
        // Check if another daemon is actually running
        if is_socket_alive(&sock_path).await {
            bail!(
                "daemon already running (socket {} is active). \
                 Use `anvil daemon stop` first.",
                sock_path.display()
            );
        }
        std::fs::remove_file(&sock_path)?;
        tracing::info!("removed stale socket: {}", sock_path.display());
    }

    // Bind the listener
    let listener = UnixListener::bind(&sock_path)?;

    // Set socket permissions to 0600 (owner only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600))?;
    }

    // Write PID file
    let pid = std::process::id();
    std::fs::write(&pid_path, pid.to_string())?;

    let start_time = Instant::now();

    eprintln!("╭─────────────────────────────────────╮");
    eprintln!("│  ⚒  Anvil Daemon v{:<17}│", env!("CARGO_PKG_VERSION"));
    eprintln!("│  listening for connections...        │");
    eprintln!("╰─────────────────────────────────────╯");
    eprintln!("  model:   {}", agent.model());
    eprintln!("  session: {}", &agent.session_id()[..8]);
    eprintln!("  socket:  {}", sock_path.display());
    eprintln!("  pid:     {pid}");
    eprintln!();

    // Task channel — all producers send here, dispatch loop consumes
    let (task_tx, mut task_rx) = mpsc::channel::<DaemonTask>(32);

    // Shared cancellation token — cancelled on shutdown to abort in-flight turns
    let shutdown_token = CancellationToken::new();

    // Spawn: UDS accept loop
    let accept_tx = task_tx.clone();
    let accept_token = shutdown_token.clone();
    let accept_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _)) => {
                            let tx = accept_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, tx).await {
                                    tracing::warn!("connection error: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            tracing::warn!("accept error: {e}");
                        }
                    }
                }
                _ = accept_token.cancelled() => {
                    tracing::info!("accept loop: shutdown");
                    break;
                }
            }
        }
    });

    // Spawn: signal handler (SIGINT / SIGTERM)
    let signal_tx = task_tx.clone();
    let signal_token = shutdown_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\n  received shutdown signal");
        signal_token.cancel(); // Cancel any in-flight turn
        let _ = signal_tx.send(DaemonTask::Shutdown).await;
    });

    // Drop our copy so the channel closes when all producers exit
    drop(task_tx);

    // === Dispatch loop ===
    // Sequential processing. The agent is exclusively owned here.
    while let Some(task) = task_rx.recv().await {
        match task {
            DaemonTask::Prompt {
                text,
                reply_tx,
                auto_approve,
            } => {
                tracing::info!("processing prompt ({} chars)", text.len());

                let cancel = shutdown_token.child_token();
                let (_perm_tx, perm_rx) = mpsc::channel::<PermissionDecision>(1);

                if auto_approve {
                    let perm_tx = _perm_tx;
                    tokio::spawn(async move {
                        loop {
                            if perm_tx.send(PermissionDecision::Allow).await.is_err() {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    });
                }
                // else: _perm_tx is dropped → recv returns None → Deny for mutating tools

                let event = Event::UserPrompt {
                    text,
                    session_id: None,
                };

                let _ = dispatch_event(&mut agent, event, &reply_tx, perm_rx, cancel).await;

                // reply_tx is dropped here, closing the channel.
                // The connection handler's recv loop exits cleanly.
            }

            DaemonTask::Status { reply_tx } => {
                let uptime = start_time.elapsed().as_secs();
                let response = Response::StatusInfo {
                    session_id: agent.session_id().to_string(),
                    model: agent.model().to_string(),
                    mode: agent.mode().to_string(),
                    uptime_secs: uptime,
                    pid,
                };
                let _ = reply_tx.send(response);
            }

            DaemonTask::Shutdown => {
                eprintln!("  shutting down...");
                shutdown_token.cancel();
                break;
            }
        }
    }

    // === Cleanup ===
    accept_handle.abort();
    agent.pause_session()?;

    if sock_path.exists() {
        std::fs::remove_file(&sock_path)?;
    }
    if pid_path.exists() {
        std::fs::remove_file(&pid_path)?;
    }

    eprintln!("  daemon stopped. session: {}", &agent.session_id()[..8]);
    Ok(())
}

/// Handle a single client connection.
///
/// Reads one `Request`, enqueues the corresponding `DaemonTask`,
/// and streams `Response` frames back to the client.
async fn handle_connection(stream: UnixStream, task_tx: mpsc::Sender<DaemonTask>) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    let request: Request = match ipc::read_frame(&mut reader).await? {
        Some(r) => r,
        None => return Ok(()), // Client disconnected before sending
    };

    match request {
        Request::Prompt { text, auto_approve } => {
            let (reply_tx, mut reply_rx) = mpsc::channel::<AgentEvent>(64);

            // Enqueue the task
            if task_tx
                .send(DaemonTask::Prompt {
                    text,
                    reply_tx,
                    auto_approve,
                })
                .await
                .is_err()
            {
                ipc::write_frame(
                    &mut writer,
                    &Response::Error {
                        message: "daemon shutting down".into(),
                    },
                )
                .await?;
                return Ok(());
            }

            // Stream agent events back as Response frames
            while let Some(event) = reply_rx.recv().await {
                let response = match event {
                    AgentEvent::ContentDelta(text) => Response::Delta { text },
                    AgentEvent::ThinkingDelta(text) => Response::Thinking { text },
                    AgentEvent::ToolCallPending {
                        name, arguments, ..
                    } => Response::ToolPending { name, arguments },
                    AgentEvent::ToolResult { name, result } => {
                        let text = result.text();
                        Response::ToolResult {
                            name,
                            lines: text.lines().count(),
                            chars: text.len(),
                        }
                    }
                    AgentEvent::TurnComplete => Response::TurnComplete,
                    AgentEvent::Error(message) => Response::Error { message },
                    AgentEvent::Cancelled => Response::Error {
                        message: "cancelled".into(),
                    },
                    // Internal events — skip over IPC
                    AgentEvent::Usage(_)
                    | AgentEvent::Retry { .. }
                    | AgentEvent::LoopDetected { .. }
                    | AgentEvent::ContextWarning { .. }
                    | AgentEvent::AutoCompacted { .. }
                    | AgentEvent::ToolOutputDelta { .. } => continue,
                };

                match tokio::time::timeout(WRITE_TIMEOUT, ipc::write_frame(&mut writer, &response))
                    .await
                {
                    Ok(Ok(())) => {} // Frame sent successfully
                    Ok(Err(_)) => {
                        // Client disconnected mid-stream
                        tracing::debug!("client disconnected, dropping connection");
                        break;
                    }
                    Err(_) => {
                        // Write timed out — client is slow or suspended.
                        // Drop the connection to free the dispatch loop.
                        tracing::warn!(
                            "write timeout ({}s), shedding connection",
                            WRITE_TIMEOUT.as_secs()
                        );
                        break;
                    }
                }
            }
        }

        Request::Status => {
            let (reply_tx, reply_rx) = oneshot::channel();

            if task_tx.send(DaemonTask::Status { reply_tx }).await.is_err() {
                ipc::write_frame(
                    &mut writer,
                    &Response::Error {
                        message: "daemon shutting down".into(),
                    },
                )
                .await?;
                return Ok(());
            }

            if let Ok(response) = reply_rx.await {
                ipc::write_frame(&mut writer, &response).await?;
            }
        }

        Request::Shutdown => {
            let _ = task_tx.send(DaemonTask::Shutdown).await;
            ipc::write_frame(&mut writer, &Response::Acknowledged).await?;
        }
    }

    Ok(())
}

/// Check if a socket file has a live daemon behind it.
/// Attempts a connection — if it succeeds, the daemon is alive.
async fn is_socket_alive(path: &std::path::Path) -> bool {
    tokio::net::UnixStream::connect(path).await.is_ok()
}
