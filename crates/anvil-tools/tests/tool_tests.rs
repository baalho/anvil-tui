use anvil_tools::ToolExecutor;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn setup() -> (TempDir, ToolExecutor) {
    let dir = TempDir::new().unwrap();
    let executor = ToolExecutor::new(dir.path().to_path_buf(), 10, 10_000);
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

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("timed out"));
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
