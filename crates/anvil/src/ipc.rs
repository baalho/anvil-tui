//! IPC wire protocol — length-prefixed JSON over Unix Domain Sockets.
//!
//! # Wire format
//! Each frame is: `[4 bytes: payload length as u32 big-endian][payload: JSON bytes]`
//!
//! Max frame size is 16 MB to prevent OOM from malformed lengths.
//! Both Request and Response use `#[serde(tag = "type")]` for clean
//! internally-tagged JSON: `{"type": "Prompt", "text": "hello"}`.
//!
//! # Why not HTTP/gRPC
//! Unix domain sockets are faster (no TCP overhead), simpler (no TLS),
//! and naturally scoped to the local machine. Length-prefixed JSON is
//! the most boring framing protocol that works.

use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Maximum frame payload size (16 MB). Prevents OOM from malformed lengths.
const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Client-to-daemon request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// Send a prompt to the agent.
    Prompt {
        text: String,
        /// Auto-approve all tool calls (no interactive permission prompts).
        auto_approve: bool,
    },
    /// Query daemon status (session, model, uptime).
    Status,
    /// Request graceful shutdown.
    Shutdown,
}

/// Daemon-to-client response. Streamed as multiple frames per request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Text content delta from the assistant.
    Delta { text: String },
    /// Thinking block delta (chain-of-thought).
    Thinking { text: String },
    /// Tool call about to execute.
    ToolPending { name: String, arguments: String },
    /// Tool execution completed.
    ToolResult {
        name: String,
        lines: usize,
        chars: usize,
    },
    /// Agent turn completed successfully.
    TurnComplete,
    /// Error occurred.
    Error { message: String },
    /// Daemon status information.
    StatusInfo {
        session_id: String,
        model: String,
        mode: String,
        uptime_secs: u64,
        watching: bool,
        pid: u32,
    },
    /// Shutdown acknowledged — daemon is stopping.
    Acknowledged,
}

/// Write a length-prefixed JSON frame to an async writer.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value)?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a length-prefixed JSON frame from an async reader.
///
/// Returns `Err` if the frame exceeds `MAX_FRAME_SIZE` or the connection
/// is closed mid-frame. Returns `Ok(None)` on clean EOF (connection closed
/// before the length prefix).
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>>
where
    R: AsyncReadExt + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }

    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_SIZE {
        bail!("frame too large: {len} bytes (max {MAX_FRAME_SIZE})");
    }

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    let value = serde_json::from_slice(&payload)?;
    Ok(Some(value))
}

/// Hash a workspace path to a short hex string for socket naming.
///
/// Uses `DefaultHasher` (SipHash) — not cryptographic, but stable within
/// a process and sufficient for disambiguating workspace paths.
fn workspace_hash(workspace: &Path) -> String {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Resolve the runtime directory for daemon files.
///
/// Prefers `$XDG_RUNTIME_DIR/anvil/` (standard on Linux).
/// Falls back to `/tmp/anvil-$UID/` (macOS, minimal systems).
fn runtime_dir() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("anvil")
    } else {
        #[cfg(unix)]
        let uid = unsafe { libc::getuid() };
        #[cfg(not(unix))]
        let uid = 0u32;
        PathBuf::from(format!("/tmp/anvil-{uid}"))
    }
}

/// Resolve the daemon socket path for a specific workspace.
///
/// Each workspace gets its own socket so multiple daemons can run
/// concurrently in different project directories:
/// `$RUNTIME_DIR/anvil/daemon-<hash>.sock`
pub fn socket_path(workspace: &Path) -> PathBuf {
    let hash = workspace_hash(workspace);
    runtime_dir().join(format!("daemon-{hash}.sock"))
}

/// Resolve the daemon PID file path for a specific workspace.
pub fn pid_path(workspace: &Path) -> PathBuf {
    let hash = workspace_hash(workspace);
    runtime_dir().join(format!("daemon-{hash}.pid"))
}

/// Create the socket directory with owner-only permissions (0700).
pub fn ensure_socket_dir(workspace: &Path) -> Result<PathBuf> {
    let dir = runtime_dir();

    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let _ = workspace; // used only for dir resolution in future
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_as_tagged_json() {
        let req = Request::Prompt {
            text: "hello".into(),
            auto_approve: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"Prompt\""));
        assert!(json.contains("\"text\":\"hello\""));
        assert!(json.contains("\"auto_approve\":true"));
    }

    #[test]
    fn response_serializes_as_tagged_json() {
        let resp = Response::Delta {
            text: "world".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"Delta\""));
        assert!(json.contains("\"text\":\"world\""));
    }

    #[test]
    fn request_roundtrip() {
        let req = Request::Shutdown;
        let json = serde_json::to_vec(&req).unwrap();
        let decoded: Request = serde_json::from_slice(&json).unwrap();
        assert!(matches!(decoded, Request::Shutdown));
    }

    #[test]
    fn response_status_roundtrip() {
        let resp = Response::StatusInfo {
            session_id: "abc123".into(),
            model: "qwen3".into(),
            mode: "coding".into(),
            uptime_secs: 42,
            watching: true,
            pid: 1234,
        };
        let json = serde_json::to_vec(&resp).unwrap();
        let decoded: Response = serde_json::from_slice(&json).unwrap();
        match decoded {
            Response::StatusInfo {
                session_id,
                uptime_secs,
                pid,
                ..
            } => {
                assert_eq!(session_id, "abc123");
                assert_eq!(uptime_secs, 42);
                assert_eq!(pid, 1234);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn frame_roundtrip() {
        let req = Request::Prompt {
            text: "test prompt".into(),
            auto_approve: false,
        };

        let mut buf = Vec::new();
        write_frame(&mut buf, &req).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded: Request = read_frame(&mut cursor).await.unwrap().unwrap();

        match decoded {
            Request::Prompt { text, auto_approve } => {
                assert_eq!(text, "test prompt");
                assert!(!auto_approve);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[tokio::test]
    async fn read_frame_returns_none_on_eof() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result: Result<Option<Request>> = read_frame(&mut cursor).await;
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn socket_path_is_absolute_and_workspace_scoped() {
        let ws = Path::new("/home/user/project-a");
        let path = socket_path(ws);
        assert!(path.is_absolute());
        assert!(path.to_string_lossy().contains("anvil"));
        assert!(path.to_string_lossy().contains("daemon-"));
        assert!(path.to_string_lossy().ends_with(".sock"));
    }

    #[test]
    fn different_workspaces_get_different_sockets() {
        let a = socket_path(Path::new("/home/user/project-a"));
        let b = socket_path(Path::new("/home/user/project-b"));
        assert_ne!(a, b);
    }

    #[test]
    fn same_workspace_gets_same_socket() {
        let a = socket_path(Path::new("/home/user/project"));
        let b = socket_path(Path::new("/home/user/project"));
        assert_eq!(a, b);
    }

    #[test]
    fn pid_path_same_directory_as_socket() {
        let ws = Path::new("/home/user/project");
        let sock = socket_path(ws);
        let pid = pid_path(ws);
        assert_eq!(sock.parent(), pid.parent());
        assert!(pid.to_string_lossy().ends_with(".pid"));
    }
}
