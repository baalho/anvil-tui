use anyhow::{bail, Result};
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
    tokio::fs::write(&path, content).await?;

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
    tokio::fs::write(&path, &new_content).await?;

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
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(per_call_timeout),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("command timed out after {per_call_timeout}s"))??;

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
