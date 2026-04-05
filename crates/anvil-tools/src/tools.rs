//! Tool implementations — 11 deterministic tools for filesystem, shell, and git.
//!
//! Each tool is an async function taking a workspace path and JSON arguments.
//! All file paths are resolved relative to the workspace root with traversal prevention.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Resolve a path relative to the workspace root, preventing traversal outside it.
fn resolve_path(workspace: &Path, path_str: &str) -> Result<PathBuf> {
    let path = Path::new(path_str);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };

    let canonical = resolved.canonicalize().or_else(|_| {
        // File might not exist yet (file_write). Canonicalize the parent.
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent)?;
            let canon_parent = parent.canonicalize()?;
            Ok(canon_parent.join(resolved.file_name().unwrap_or_default()))
        } else {
            bail!("cannot resolve path: {path_str}")
        }
    })?;

    let workspace_canonical = workspace.canonicalize()?;
    if !canonical.starts_with(&workspace_canonical) {
        bail!(
            "path escapes workspace boundary: {} is outside {}",
            canonical.display(),
            workspace_canonical.display()
        );
    }

    Ok(canonical)
}

/// Write to a file with TOCTOU protection.
///
/// On Unix, re-verifies the path hasn't been replaced with a symlink between
/// `resolve_path` and the actual write. For existing files, opens with
/// `O_NOFOLLOW` to reject symlinks. For new files, re-canonicalizes the
/// parent directory and verifies it's still within the workspace.
async fn safe_write(path: &Path, content: &[u8], workspace: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Try O_NOFOLLOW first — works for existing files, rejects symlinks
        let result = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path);

        match result {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(content)?;
                Ok(())
            }
            Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                bail!("refusing to write through symlink: {}", path.display());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // New file — verify parent is still within workspace
                if let Some(parent) = path.parent() {
                    let canon_parent = parent.canonicalize()?;
                    let canon_ws = workspace.canonicalize()?;
                    if !canon_parent.starts_with(&canon_ws) {
                        bail!(
                            "parent directory escaped workspace: {}",
                            canon_parent.display()
                        );
                    }
                }
                tokio::fs::write(path, content).await?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    #[cfg(not(unix))]
    {
        tokio::fs::write(path, content).await?;
        Ok(())
    }
}

pub async fn file_read(workspace: &Path, args: &Value) -> Result<String> {
    let path_str = args["path"].as_str().unwrap_or_default();
    let path = resolve_path(workspace, path_str)?;

    let content = tokio::fs::read_to_string(&path).await?;
    let lines: Vec<&str> = content.lines().collect();

    let start = args["start_line"]
        .as_u64()
        .map(|n| (n as usize).saturating_sub(1))
        .unwrap_or(0);
    let end = args["end_line"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(lines.len());

    let selected: Vec<String> = lines
        .iter()
        .enumerate()
        .skip(start)
        .take(end - start)
        .map(|(i, line)| format!("{:>4} | {}", i + 1, line))
        .collect();

    Ok(selected.join("\n"))
}

pub async fn file_write(workspace: &Path, args: &Value) -> Result<String> {
    let path_str = args["path"].as_str().unwrap_or_default();
    let content = args["content"].as_str().unwrap_or_default();
    let path = resolve_path(workspace, path_str)?;

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    safe_write(&path, content.as_bytes(), workspace)
        .await
        .with_context(|| format!("writing {}", path.display()))?;

    Ok(format!(
        "wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

pub async fn file_edit(workspace: &Path, args: &Value) -> Result<String> {
    let path_str = args["path"].as_str().unwrap_or_default();
    let old_str = args["old_str"].as_str().unwrap_or_default();
    let new_str = args["new_str"].as_str().unwrap_or("");
    let path = resolve_path(workspace, path_str)?;

    let content = tokio::fs::read_to_string(&path).await?;
    let count = content.matches(old_str).count();

    if count == 0 {
        bail!("old_str not found in {}", path.display());
    }
    if count > 1 {
        bail!(
            "old_str matches {} locations in {} — must be unique",
            count,
            path.display()
        );
    }

    let new_content = content.replacen(old_str, new_str, 1);
    safe_write(&path, new_content.as_bytes(), workspace)
        .await
        .with_context(|| format!("editing {}", path.display()))?;

    Ok(format!("edited {}", path.display()))
}

/// Execute a shell command with environment passthrough from active skills.
///
/// # Security model
/// The shell starts with `env_clear()` and only passes through:
/// 1. Base safe vars (PATH, HOME, USER, LANG, TERM + platform-specific)
/// 2. Extra vars declared by active skills' frontmatter (`env: [DOCKER_HOST]`)
///
/// This prevents accidental leakage of secrets while allowing infrastructure
/// skills to function. The user must consciously activate a skill to enable
/// its env vars.
pub async fn shell(
    workspace: &Path,
    args: &Value,
    timeout_secs: u64,
    extra_env: &[String],
) -> Result<String> {
    let command_str = args["command"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("command must be a string"))?;

    if command_str.trim().is_empty() {
        bail!("command is empty");
    }

    let per_call_timeout = args["timeout"].as_u64().unwrap_or(timeout_secs);

    let working_dir = args["working_dir"]
        .as_str()
        .map(|d| resolve_path(workspace, d))
        .transpose()?
        .unwrap_or_else(|| workspace.to_path_buf());

    #[cfg(unix)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command_str);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd.exe");
        c.arg("/C").arg(command_str);
        c
    };

    cmd.current_dir(&working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();

    #[cfg(unix)]
    const SAFE_VARS: &[&str] = &["PATH", "HOME", "USER", "LANG", "TERM"];
    #[cfg(windows)]
    const SAFE_VARS: &[&str] = &[
        "PATH",
        "SYSTEMROOT",
        "USERPROFILE",
        "USERNAME",
        "TEMP",
        "TMP",
        "COMSPEC",
    ];

    for var in SAFE_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    // Pass through extra env vars from active skills (e.g. DOCKER_HOST, SSH_AUTH_SOCK)
    for var in extra_env {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    let child = cmd.spawn()?;

    // Capture the PID before wait_with_output() takes ownership
    #[cfg(unix)]
    let child_pid = child.id();

    let timeout_duration = std::time::Duration::from_secs(per_call_timeout);
    match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            let mut result = format!("exit code: {exit_code}\n");
            if !stdout.is_empty() {
                result.push_str(&format!("stdout:\n{stdout}"));
            }
            if !stderr.is_empty() {
                result.push_str(&format!("stderr:\n{stderr}"));
            }
            Ok(result)
        }
        Ok(Err(e)) => bail!("command failed: {e}"),
        Err(_) => {
            // Timeout: try SIGTERM first, then SIGKILL after 5s.
            // Note: wait_with_output() consumed `child`, so we use the
            // saved PID to send signals directly.
            let result = format!("error: command timed out after {per_call_timeout}s (killed)\n");

            #[cfg(unix)]
            if let Some(pid) = child_pid {
                // Send SIGTERM to the process group
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                // Give it 5s to exit, then SIGKILL
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }

            #[cfg(not(unix))]
            {
                // On non-Unix, the dropped child handle will terminate the process
                let _ = per_call_timeout; // suppress unused warning
            }

            Ok(result)
        }
    }
}

pub async fn grep(workspace: &Path, args: &Value) -> Result<String> {
    let pattern_str = args["pattern"].as_str().unwrap_or_default();
    let path_str = args["path"].as_str().unwrap_or_default();
    let include = args["include"].as_str();
    let path = resolve_path(workspace, path_str)?;

    let pattern = regex::Regex::new(pattern_str)?;
    let mut results = Vec::new();

    if path.is_file() {
        grep_file(&path, &pattern, &mut results).await?;
    } else if path.is_dir() {
        grep_dir(&path, &pattern, include, &mut results, 0).await?;
    }

    if results.is_empty() {
        Ok("no matches found".to_string())
    } else {
        Ok(results.join("\n"))
    }
}

async fn grep_file(path: &Path, pattern: &regex::Regex, results: &mut Vec<String>) -> Result<()> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return Ok(()), // skip binary/unreadable files
    };

    for (i, line) in content.lines().enumerate() {
        if pattern.is_match(line) {
            results.push(format!("{}:{}: {}", path.display(), i + 1, line));
        }
    }
    Ok(())
}

async fn grep_dir(
    dir: &Path,
    pattern: &regex::Regex,
    include: Option<&str>,
    results: &mut Vec<String>,
    depth: usize,
) -> Result<()> {
    if depth > 10 {
        return Ok(());
    }

    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden dirs and common noise
        if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
            continue;
        }

        if path.is_dir() {
            Box::pin(grep_dir(&path, pattern, include, results, depth + 1)).await?;
        } else if path.is_file() {
            if let Some(glob) = include {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                if !glob_match(glob, &file_name) {
                    continue;
                }
            }
            grep_file(&path, pattern, results).await?;
            if results.len() > 500 {
                results.push("... (truncated, too many matches)".to_string());
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Simple glob matching supporting `*` wildcards.
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        return name.ends_with(&format!(".{ext}"));
    }
    name == pattern
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "dist",
    "build",
];

fn should_skip(name: &str, include_hidden: bool) -> bool {
    if SKIP_DIRS.contains(&name) {
        return true;
    }
    if !include_hidden && name.starts_with('.') {
        return true;
    }
    false
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub async fn ls(workspace: &Path, args: &Value) -> Result<String> {
    let path_str = args["path"].as_str().unwrap_or(".");
    let include_all = args["all"].as_bool().unwrap_or(false);
    let path = resolve_path(workspace, path_str)?;

    if !path.is_dir() {
        bail!("{} is not a directory", path.display());
    }

    let mut entries = tokio::fs::read_dir(&path).await?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();

        if should_skip(&name_str, include_all) {
            continue;
        }

        let metadata = entry.metadata().await?;
        if metadata.is_dir() {
            dirs.push(format!("dir   {name_str}/"));
        } else {
            let size = format_size(metadata.len());
            files.push(format!("file  {name_str}  ({size})"));
        }
    }

    dirs.sort();
    files.sort();

    let mut result = dirs;
    result.extend(files);

    if result.is_empty() {
        Ok("(empty directory)".to_string())
    } else {
        Ok(result.join("\n"))
    }
}

pub async fn find(workspace: &Path, args: &Value) -> Result<String> {
    let pattern = args["pattern"].as_str().unwrap_or("*");
    let path_str = args["path"].as_str().unwrap_or(".");
    let max_depth = args["max_depth"].as_u64().unwrap_or(10) as usize;
    let path = resolve_path(workspace, path_str)?;

    if !path.is_dir() {
        bail!("{} is not a directory", path.display());
    }

    let mut results = Vec::new();
    find_recursive(&path, &path, pattern, &mut results, 0, max_depth).await?;

    if results.is_empty() {
        Ok("no files found".to_string())
    } else if results.len() > 500 {
        results.truncate(500);
        results.push("... (truncated, over 500 matches)".to_string());
        Ok(results.join("\n"))
    } else {
        Ok(results.join("\n"))
    }
}

async fn find_recursive(
    base: &Path,
    dir: &Path,
    pattern: &str,
    results: &mut Vec<String>,
    depth: usize,
    max_depth: usize,
) -> Result<()> {
    if depth > max_depth || results.len() > 500 {
        return Ok(());
    }

    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if should_skip(&name_str, false) {
            continue;
        }

        let path = entry.path();
        if path.is_dir() {
            Box::pin(find_recursive(
                base,
                &path,
                pattern,
                results,
                depth + 1,
                max_depth,
            ))
            .await?;
        } else if glob_match(pattern, &name_str) {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            results.push(relative);
        }
    }
    Ok(())
}

// --- Git tools ---
// Purpose-built git operations that avoid the shell tool's env_clear() restrictions
// and provide structured output.

/// Run `git status` in the workspace. Returns short-format status.
pub async fn git_status(workspace: &Path, args: &Value) -> Result<String> {
    let verbose = args["verbose"].as_bool().unwrap_or(false);

    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(workspace);

    if verbose {
        cmd.args(["status"]);
    } else {
        cmd.args(["status", "--short", "--branch"]);
    }

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        bail!("git status failed: {stderr}");
    }

    if stdout.trim().is_empty() {
        Ok("working tree clean".to_string())
    } else {
        Ok(stdout.to_string())
    }
}

/// Run `git diff` in the workspace. Supports staged, unstaged, and ref comparisons.
pub async fn git_diff(workspace: &Path, args: &Value) -> Result<String> {
    let staged = args["staged"].as_bool().unwrap_or(false);
    let path = args["path"].as_str();
    let ref_spec = args["ref"].as_str();

    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(workspace).arg("diff");

    if staged {
        cmd.arg("--cached");
    }

    if let Some(r) = ref_spec {
        cmd.arg(r);
    }

    cmd.arg("--stat").arg("--patch");

    if let Some(p) = path {
        cmd.arg("--").arg(p);
    }

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        bail!("git diff failed: {stderr}");
    }

    if stdout.trim().is_empty() {
        Ok("no differences".to_string())
    } else {
        Ok(stdout.to_string())
    }
}

/// Run `git log` in the workspace. Returns recent commits.
pub async fn git_log(workspace: &Path, args: &Value) -> Result<String> {
    let count = args["count"].as_u64().unwrap_or(10);
    let oneline = args["oneline"].as_bool().unwrap_or(true);
    let path = args["path"].as_str();

    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(workspace).arg("log");

    cmd.arg(format!("-{count}"));

    if oneline {
        cmd.arg("--oneline");
    } else {
        cmd.arg("--format=%H %an <%ae> %ai%n  %s");
    }

    if let Some(p) = path {
        cmd.arg("--").arg(p);
    }

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        bail!("git log failed: {stderr}");
    }

    if stdout.trim().is_empty() {
        Ok("no commits".to_string())
    } else {
        Ok(stdout.to_string())
    }
}

/// Run `git commit` in the workspace. Stages specified files and commits.
///
/// This is a mutating operation — requires user permission.
pub async fn git_commit(workspace: &Path, args: &Value) -> Result<String> {
    let message = args["message"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("commit message is required"))?;

    let files = args["files"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let all = args["all"].as_bool().unwrap_or(false);

    // Stage files
    if !files.is_empty() {
        let mut add_cmd = tokio::process::Command::new("git");
        add_cmd.current_dir(workspace).arg("add");
        for f in &files {
            add_cmd.arg(f);
        }
        let add_output = add_cmd.output().await?;
        if !add_output.status.success() {
            let stderr = String::from_utf8_lossy(&add_output.stderr);
            bail!("git add failed: {stderr}");
        }
    }

    // Commit
    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(workspace).arg("commit");

    if all && files.is_empty() {
        cmd.arg("-a");
    }

    cmd.arg("-m").arg(message);

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        if stdout.contains("nothing to commit") || stderr.contains("nothing to commit") {
            return Ok("nothing to commit, working tree clean".to_string());
        }
        bail!("git commit failed: {stderr}");
    }

    Ok(stdout.to_string())
}
