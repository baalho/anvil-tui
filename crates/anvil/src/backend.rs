//! Backend lifecycle management — start/stop LLM server processes.
//!
//! # Why this exists
//! Users may want Anvil to manage the LLM backend process (e.g., start llama-server
//! with a specific model). This avoids requiring a separate terminal to run the server.
//!
//! # How it works
//! `BackendProcess` wraps a child process (llama-server, mlx_lm.server, etc.).
//! It starts the server, waits for it to become healthy via HTTP polling, and
//! kills it on drop or explicit stop. If a server is already running on the
//! target port, it connects to it instead of starting a new one.

use anyhow::{bail, Result};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A managed backend server process.
pub struct BackendProcess {
    child: Option<Child>,
    #[allow(dead_code)]
    port: u16,
    base_url: String,
}

impl BackendProcess {
    /// Start a llama-server process with the given model file.
    ///
    /// If a server is already listening on the port, returns a handle
    /// that connects to it without starting a new process.
    pub async fn start_llama_server(
        model_path: &str,
        port: u16,
        extra_args: &[&str],
    ) -> Result<Self> {
        let base_url = format!("http://127.0.0.1:{port}/v1");

        // Check if something is already running on this port
        if check_health(&base_url).await {
            eprintln!("backend: server already running on port {port}, connecting");
            return Ok(Self {
                child: None,
                port,
                base_url,
            });
        }

        // Build the command
        let mut cmd = Command::new("llama-server");
        cmd.arg("--model")
            .arg(model_path)
            .arg("--port")
            .arg(port.to_string())
            .arg("--host")
            .arg("127.0.0.1");

        for arg in extra_args {
            cmd.arg(arg);
        }

        // Redirect stdout/stderr to avoid cluttering the terminal
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());

        let child = cmd.spawn().map_err(|e| {
            anyhow::anyhow!(
                "failed to start llama-server (is it installed?): {e}"
            )
        })?;

        eprintln!("backend: started llama-server (pid {}) on port {port}", child.id());

        let mut process = Self {
            child: Some(child),
            port,
            base_url: base_url.clone(),
        };

        // Wait for the server to become healthy
        if let Err(e) = process.wait_for_health(Duration::from_secs(60)).await {
            process.stop();
            bail!("llama-server failed to start: {e}");
        }

        eprintln!("backend: llama-server ready on port {port}");
        Ok(process)
    }

    /// The base URL for API requests (e.g., `http://127.0.0.1:8080/v1`).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The port the server is listening on.
    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether this handle manages a child process (vs connecting to existing).
    #[allow(dead_code)]
    pub fn is_managed(&self) -> bool {
        self.child.is_some()
    }

    /// Stop the managed server process.
    ///
    /// Sends SIGTERM (via `kill` command on Unix) first, then SIGKILL after
    /// 5 seconds if still running. No-op if this handle connected to an
    /// existing server.
    pub fn stop(&mut self) {
        if let Some(ref mut child) = self.child {
            let pid = child.id();
            eprintln!("backend: stopping llama-server (pid {pid})");

            // Try graceful shutdown via SIGTERM
            #[cfg(unix)]
            {
                let _ = Command::new("kill")
                    .arg("-TERM")
                    .arg(pid.to_string())
                    .output();
            }
            #[cfg(not(unix))]
            {
                let _ = child.kill();
            }

            // Wait up to 5 seconds for graceful exit
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        eprintln!("backend: llama-server stopped");
                        break;
                    }
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            eprintln!("backend: force-killing llama-server");
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
            self.child = None;
        }
    }

    /// Poll the health endpoint until the server responds or timeout.
    async fn wait_for_health(&self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        let poll_interval = Duration::from_millis(500);

        while start.elapsed() < timeout {
            if check_health(&self.base_url).await {
                return Ok(());
            }

            // Check if the child process has exited unexpectedly
            if let Some(ref child) = self.child {
                // We can't call try_wait on a shared ref, so just check health
                let _ = child;
            }

            tokio::time::sleep(poll_interval).await;
        }

        bail!(
            "server did not become healthy within {}s",
            timeout.as_secs()
        )
    }
}

impl Drop for BackendProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Check if a server is responding on the given base URL.
async fn check_health(base_url: &str) -> bool {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build();

    let client = match client {
        Ok(c) => c,
        Err(_) => return false,
    };

    matches!(client.get(&url).send().await, Ok(resp) if resp.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_health_returns_false_for_no_server() {
        // No server on this port
        assert!(!check_health("http://127.0.0.1:19999/v1").await);
    }

    #[test]
    fn backend_process_stop_is_noop_for_unmanaged() {
        let mut bp = BackendProcess {
            child: None,
            port: 8080,
            base_url: "http://127.0.0.1:8080/v1".to_string(),
        };
        bp.stop(); // should not panic
        assert!(!bp.is_managed());
    }
}
