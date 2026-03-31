use anvil_tools::ToolExecutor;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn setup() -> (TempDir, ToolExecutor) {
    let dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(dir.path().to_path_buf(), 10, 10_000);
    (dir, executor)
}

/// Create a temp dir with an initialized git repo and one commit.
fn setup_git() -> (TempDir, ToolExecutor) {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(path)
        .output()
        .unwrap();

    fs::write(path.join("README.md"), "# Test\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(path)
        .output()
        .unwrap();

    let executor = ToolExecutor::new(path.to_path_buf(), 10, 10_000);
    (dir, executor)
}

#[tokio::test]
async fn file_write_and_read() {
    let (_dir, executor) = setup();

    let write_result = executor
        .execute(
            "file_write",
            &json!({"path": "test.txt", "content": "hello world"}),
        )
        .await
        .unwrap();
    assert!(write_result.contains("wrote"));

    let read_result = executor
        .execute("file_read", &json!({"path": "test.txt"}))
        .await
        .unwrap();
    assert!(read_result.contains("hello world"));
}

#[tokio::test]
async fn file_read_with_line_range() {
    let (dir, executor) = setup();
    fs::write(
        dir.path().join("lines.txt"),
        "line1\nline2\nline3\nline4\nline5",
    )
    .unwrap();

    let result = executor
        .execute(
            "file_read",
            &json!({"path": "lines.txt", "start_line": 2, "end_line": 4}),
        )
        .await
        .unwrap();
    assert!(result.contains("line2"));
    assert!(result.contains("line4"));
    assert!(!result.contains("line1"));
    assert!(!result.contains("line5"));
}

#[tokio::test]
async fn file_edit_replaces_unique_match() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("edit.txt"), "foo bar baz").unwrap();

    let result = executor
        .execute(
            "file_edit",
            &json!({"path": "edit.txt", "old_str": "bar", "new_str": "qux"}),
        )
        .await
        .unwrap();
    assert!(result.contains("edited"));

    let content = fs::read_to_string(dir.path().join("edit.txt")).unwrap();
    assert_eq!(content, "foo qux baz");
}

#[tokio::test]
async fn file_edit_fails_on_no_match() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("edit.txt"), "foo bar baz").unwrap();

    let result = executor
        .execute(
            "file_edit",
            &json!({"path": "edit.txt", "old_str": "nonexistent", "new_str": "x"}),
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn file_edit_fails_on_multiple_matches() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("edit.txt"), "foo foo foo").unwrap();

    let result = executor
        .execute(
            "file_edit",
            &json!({"path": "edit.txt", "old_str": "foo", "new_str": "bar"}),
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn file_edit_delete_when_new_str_omitted() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("edit.txt"), "keep remove keep").unwrap();

    executor
        .execute(
            "file_edit",
            &json!({"path": "edit.txt", "old_str": " remove"}),
        )
        .await
        .unwrap();

    let content = fs::read_to_string(dir.path().join("edit.txt")).unwrap();
    assert_eq!(content, "keep keep");
}

#[tokio::test]
async fn shell_executes_string_command() {
    let (_dir, executor) = setup();

    let result = executor
        .execute("shell", &json!({"command": "echo hello"}))
        .await
        .unwrap();
    assert!(result.contains("hello"));
    assert!(result.contains("exit code: 0"));
}

#[tokio::test]
async fn shell_with_pipes_and_chains() {
    let (_dir, executor) = setup();

    #[cfg(unix)]
    let result = executor
        .execute("shell", &json!({"command": "echo hello && echo world"}))
        .await
        .unwrap();
    #[cfg(windows)]
    let result = executor
        .execute("shell", &json!({"command": "echo hello & echo world"}))
        .await
        .unwrap();

    assert!(result.contains("hello"));
    assert!(result.contains("world"));
}

#[tokio::test]
async fn shell_reports_nonzero_exit() {
    let (_dir, executor) = setup();

    #[cfg(unix)]
    let result = executor
        .execute("shell", &json!({"command": "false"}))
        .await
        .unwrap();
    #[cfg(windows)]
    let result = executor
        .execute("shell", &json!({"command": "exit /b 1"}))
        .await
        .unwrap();

    assert!(result.contains("exit code: 1"));
}

#[tokio::test]
async fn shell_rejects_empty_command() {
    let (_dir, executor) = setup();
    let result = executor.execute("shell", &json!({"command": "  "})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn shell_per_call_timeout() {
    let (_dir, executor) = setup();

    #[cfg(unix)]
    let result = executor
        .execute("shell", &json!({"command": "sleep 60", "timeout": 1}))
        .await;
    #[cfg(windows)]
    let result = executor
        .execute("shell", &json!({"command": "timeout /t 60", "timeout": 1}))
        .await;

    // Timeout returns Ok with an error message (not Err), so the LLM
    // gets the timeout info as a tool result for self-correction.
    let output = result.unwrap();
    assert!(output.contains("timed out"), "got: {output}");
    assert!(output.contains("killed"), "got: {output}");
}

#[tokio::test]
async fn grep_finds_matches() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("search.txt"), "apple\nbanana\napricot\n").unwrap();

    let result = executor
        .execute("grep", &json!({"pattern": "^ap", "path": "search.txt"}))
        .await
        .unwrap();
    assert!(result.contains("apple"));
    assert!(result.contains("apricot"));
    assert!(!result.contains("banana"));
}

#[tokio::test]
async fn grep_no_matches() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("search.txt"), "apple\nbanana\n").unwrap();

    let result = executor
        .execute("grep", &json!({"pattern": "cherry", "path": "search.txt"}))
        .await
        .unwrap();
    assert!(result.contains("no matches"));
}

#[tokio::test]
async fn grep_directory_with_include() {
    let (dir, executor) = setup();
    let sub = dir.path().join("src");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("main.rs"), "fn main() {}\n").unwrap();
    fs::write(sub.join("notes.txt"), "fn notes() {}\n").unwrap();

    let result = executor
        .execute(
            "grep",
            &json!({"pattern": "fn", "path": "src", "include": "*.rs"}),
        )
        .await
        .unwrap();
    assert!(result.contains("main.rs"));
    assert!(!result.contains("notes.txt"));
}

#[tokio::test]
async fn path_traversal_blocked() {
    let (_, executor) = setup();

    let result = executor
        .execute("file_read", &json!({"path": "../../../etc/passwd"}))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("escapes workspace boundary"));
}

#[tokio::test]
async fn output_truncation_with_temp_file() {
    let dir = TempDir::new().unwrap();
    // Set max_bytes low to trigger truncation
    let executor = ToolExecutor::new(dir.path().to_path_buf(), 10, 50);
    let lines: Vec<String> = (1..=300).map(|i| format!("line {i}")).collect();
    let long_content = lines.join("\n");
    fs::write(dir.path().join("long.txt"), &long_content).unwrap();

    let result = executor
        .execute("file_read", &json!({"path": "long.txt"}))
        .await
        .unwrap();
    assert!(result.contains("[Showing lines"));
    assert!(result.contains("Full output:"));
}

#[tokio::test]
async fn ls_lists_directory() {
    let (dir, executor) = setup();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
    fs::write(dir.path().join("README.md"), "# Hello").unwrap();

    let result = executor.execute("ls", &json!({"path": "."})).await.unwrap();
    assert!(result.contains("dir   src/"));
    assert!(result.contains("main.rs"));
    assert!(result.contains("README.md"));
}

#[tokio::test]
async fn ls_skips_hidden_by_default() {
    let (dir, executor) = setup();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join("visible.txt"), "hi").unwrap();

    let result = executor.execute("ls", &json!({"path": "."})).await.unwrap();
    assert!(result.contains("visible.txt"));
    assert!(!result.contains(".git"));
}

#[tokio::test]
async fn ls_shows_hidden_with_all_flag() {
    let (dir, executor) = setup();
    fs::write(dir.path().join(".hidden"), "secret").unwrap();
    fs::write(dir.path().join("visible.txt"), "hi").unwrap();

    let result = executor
        .execute("ls", &json!({"path": ".", "all": true}))
        .await
        .unwrap();
    assert!(result.contains("visible.txt"));
    assert!(result.contains(".hidden"));
}

#[tokio::test]
async fn ls_outside_workspace_blocked() {
    let (_dir, executor) = setup();
    let result = executor.execute("ls", &json!({"path": "../../.."})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn find_matches_glob() {
    let (dir, executor) = setup();
    let sub = dir.path().join("src");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("main.rs"), "fn main() {}").unwrap();
    fs::write(sub.join("lib.rs"), "pub mod lib;").unwrap();
    fs::write(sub.join("notes.txt"), "notes").unwrap();

    let result = executor
        .execute("find", &json!({"pattern": "*.rs", "path": "."}))
        .await
        .unwrap();
    assert!(result.contains("main.rs"));
    assert!(result.contains("lib.rs"));
    assert!(!result.contains("notes.txt"));
}

#[tokio::test]
async fn find_no_matches() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("file.txt"), "hello").unwrap();

    let result = executor
        .execute("find", &json!({"pattern": "*.xyz"}))
        .await
        .unwrap();
    assert!(result.contains("no files found"));
}

#[tokio::test]
async fn find_outside_workspace_blocked() {
    let (_dir, executor) = setup();
    let result = executor
        .execute("find", &json!({"pattern": "*", "path": "../../.."}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn unknown_tool_fails() {
    let (_, executor) = setup();
    let result = executor.execute("nonexistent", &json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn validate_missing_required_args() {
    let (_, executor) = setup();

    // file_read requires "path"
    let result = executor.execute("file_read", &json!({})).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("missing required"), "got: {err}");
    assert!(err.contains("path"), "got: {err}");

    // file_write requires "path" and "content"
    let result = executor
        .execute("file_write", &json!({"path": "test.txt"}))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("content"), "got: {err}");

    // shell requires "command"
    let result = executor.execute("shell", &json!({})).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("command"), "got: {err}");
}

#[tokio::test]
async fn validate_empty_string_args_rejected() {
    let (_, executor) = setup();

    let result = executor.execute("file_read", &json!({"path": ""})).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("missing required"), "got: {err}");
}

#[tokio::test]
async fn validate_null_args_rejected() {
    let (_, executor) = setup();

    let result = executor.execute("shell", &json!({"command": null})).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("missing required"), "got: {err}");
}

// === File cache tests ===

#[tokio::test]
async fn file_cache_invalidated_on_write() {
    let (dir, executor) = setup();
    let path = dir.path().join("cached.txt");
    fs::write(&path, "original").unwrap();

    let r1 = executor
        .execute("file_read", &json!({"path": "cached.txt"}))
        .await
        .unwrap();
    assert!(r1.contains("original"));

    executor
        .execute(
            "file_write",
            &json!({"path": "cached.txt", "content": "updated"}),
        )
        .await
        .unwrap();

    let r2 = executor
        .execute("file_read", &json!({"path": "cached.txt"}))
        .await
        .unwrap();
    assert!(r2.contains("updated"), "got: {r2}");
}

#[tokio::test]
async fn file_cache_invalidated_on_edit() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("edit_me.txt"), "hello world").unwrap();

    let _ = executor
        .execute("file_read", &json!({"path": "edit_me.txt"}))
        .await
        .unwrap();

    executor
        .execute(
            "file_edit",
            &json!({"path": "edit_me.txt", "old_str": "hello", "new_str": "goodbye"}),
        )
        .await
        .unwrap();

    let result = executor
        .execute("file_read", &json!({"path": "edit_me.txt"}))
        .await
        .unwrap();
    assert!(result.contains("goodbye"), "got: {result}");
}

// === Permission handler tests ===

#[test]
fn is_read_only_classification() {
    use anvil_tools::PermissionHandler;

    assert!(PermissionHandler::is_read_only("file_read"));
    assert!(PermissionHandler::is_read_only("grep"));
    assert!(PermissionHandler::is_read_only("ls"));
    assert!(PermissionHandler::is_read_only("find"));

    assert!(!PermissionHandler::is_read_only("shell"));
    assert!(!PermissionHandler::is_read_only("file_write"));
    assert!(!PermissionHandler::is_read_only("file_edit"));
    assert!(!PermissionHandler::is_read_only("unknown_tool"));
}

// === Plugin template rendering tests ===

#[test]
fn plugin_render_numeric_args() {
    let plugin: anvil_tools::ToolPlugin = toml::from_str(
        r#"
        name = "scale"
        description = "Scale by factor"
        [[params]]
        name = "factor"
        type = "number"
        required = true
        [command]
        template = "scale --factor {{factor}}"
        "#,
    )
    .unwrap();

    let cmd = plugin.render_command(&json!({"factor": 2.5})).unwrap();
    assert_eq!(cmd, "scale --factor 2.5");
}

#[test]
fn plugin_render_missing_arg_leaves_placeholder() {
    let plugin: anvil_tools::ToolPlugin = toml::from_str(
        r#"
        name = "greet"
        description = "Greet"
        [command]
        template = "echo {{name}}"
        "#,
    )
    .unwrap();

    let cmd = plugin.render_command(&json!({})).unwrap();
    assert_eq!(cmd, "echo {{name}}");
}

// === Hook tests ===

#[tokio::test]
async fn post_hook_runs_after_tool() {
    let dir = TempDir::new().unwrap();
    let hooks_dir = dir.path().join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("post-file_write.sh"),
        "#!/bin/sh\necho post-hook-executed",
    )
    .unwrap();

    let runner = anvil_tools::HookRunner::new(hooks_dir);
    let result = runner.run_post_hook("file_write").await;
    assert!(result.ran);
    assert!(result.success);
    assert!(result.output.contains("post-hook-executed"));
}

#[tokio::test]
async fn hook_nonexistent_dir_is_safe() {
    let runner = anvil_tools::HookRunner::new(std::path::PathBuf::from("/nonexistent/hooks"));
    let result = runner.run_pre_hook("shell").await;
    assert!(!result.ran);
    assert!(result.success);
}

// === Validation edge cases ===

#[tokio::test]
async fn validate_numeric_args_accepted() {
    let (dir, executor) = setup();
    fs::write(dir.path().join("data.txt"), "line1\nline2\nline3\n").unwrap();

    let result = executor
        .execute(
            "file_read",
            &json!({"path": "data.txt", "start_line": 2, "end_line": 3}),
        )
        .await
        .unwrap();
    assert!(result.contains("line2"), "got: {result}");
}

#[tokio::test]
async fn validate_boolean_args_accepted() {
    let (dir, executor) = setup();
    fs::create_dir_all(dir.path().join(".hidden")).unwrap();
    fs::write(dir.path().join(".hidden/secret.txt"), "x").unwrap();

    let result = executor
        .execute("ls", &json!({"path": ".", "all": true}))
        .await
        .unwrap();
    assert!(!result.is_empty());
}

// --- Git tool tests ---

#[tokio::test]
async fn git_status_clean_repo() {
    let (_dir, executor) = setup_git();
    let result = executor.execute("git_status", &json!({})).await.unwrap();
    // Branch info is present, no modified files
    assert!(result.contains("master") || result.contains("main"));
}

#[tokio::test]
async fn git_status_with_changes() {
    let (dir, executor) = setup_git();
    fs::write(dir.path().join("new.txt"), "new file").unwrap();

    let result = executor.execute("git_status", &json!({})).await.unwrap();
    assert!(result.contains("new.txt"));
}

#[tokio::test]
async fn git_status_verbose() {
    let (_dir, executor) = setup_git();
    let result = executor
        .execute("git_status", &json!({"verbose": true}))
        .await
        .unwrap();
    assert!(result.contains("nothing to commit"));
}

#[tokio::test]
async fn git_diff_no_changes() {
    let (_dir, executor) = setup_git();
    let result = executor.execute("git_diff", &json!({})).await.unwrap();
    assert_eq!(result, "no differences");
}

#[tokio::test]
async fn git_diff_unstaged_changes() {
    let (dir, executor) = setup_git();
    fs::write(dir.path().join("README.md"), "# Updated\n").unwrap();

    let result = executor.execute("git_diff", &json!({})).await.unwrap();
    assert!(result.contains("README.md"));
    assert!(result.contains("Updated"));
}

#[tokio::test]
async fn git_diff_staged() {
    let (dir, executor) = setup_git();
    fs::write(dir.path().join("README.md"), "# Staged\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let result = executor
        .execute("git_diff", &json!({"staged": true}))
        .await
        .unwrap();
    assert!(result.contains("Staged"));
}

#[tokio::test]
async fn git_log_shows_commits() {
    let (_dir, executor) = setup_git();
    let result = executor.execute("git_log", &json!({})).await.unwrap();
    assert!(result.contains("initial"));
}

#[tokio::test]
async fn git_log_with_count() {
    let (_dir, executor) = setup_git();
    let result = executor
        .execute("git_log", &json!({"count": 1}))
        .await
        .unwrap();
    let lines: Vec<&str> = result.trim().lines().collect();
    assert_eq!(lines.len(), 1);
}

#[tokio::test]
async fn git_log_detailed_format() {
    let (_dir, executor) = setup_git();
    let result = executor
        .execute("git_log", &json!({"oneline": false}))
        .await
        .unwrap();
    assert!(result.contains("test@test.com"));
}

#[tokio::test]
async fn git_commit_with_files() {
    let (dir, executor) = setup_git();
    fs::write(dir.path().join("new.txt"), "content").unwrap();

    let result = executor
        .execute(
            "git_commit",
            &json!({"message": "add new file", "files": ["new.txt"]}),
        )
        .await
        .unwrap();
    assert!(result.contains("add new file"));

    // Verify commit is in log
    let log = executor.execute("git_log", &json!({})).await.unwrap();
    assert!(log.contains("add new file"));
}

#[tokio::test]
async fn git_commit_all_flag() {
    let (dir, executor) = setup_git();
    fs::write(dir.path().join("README.md"), "# Changed\n").unwrap();

    let result = executor
        .execute(
            "git_commit",
            &json!({"message": "update readme", "all": true}),
        )
        .await
        .unwrap();
    assert!(result.contains("update readme"));
}

#[tokio::test]
async fn git_commit_nothing_to_commit() {
    let (_dir, executor) = setup_git();
    let result = executor
        .execute("git_commit", &json!({"message": "empty"}))
        .await
        .unwrap();
    assert!(result.contains("nothing to commit"));
}

#[tokio::test]
async fn git_commit_requires_message() {
    let (_dir, executor) = setup_git();
    let result = executor.execute("git_commit", &json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn git_tools_read_only_classification() {
    assert!(anvil_tools::PermissionHandler::is_read_only("git_status"));
    assert!(anvil_tools::PermissionHandler::is_read_only("git_diff"));
    assert!(anvil_tools::PermissionHandler::is_read_only("git_log"));
    assert!(!anvil_tools::PermissionHandler::is_read_only("git_commit"));
}
