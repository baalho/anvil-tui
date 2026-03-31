use anvil_agent::{SessionStatus, SessionStore, ToolCallEntry};
use tempfile::TempDir;

fn setup() -> (TempDir, SessionStore) {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let store = SessionStore::open(&db_path).unwrap();
    (dir, store)
}

#[test]
fn create_and_list_sessions() {
    let (_dir, store) = setup();

    let s1 = store.create_session().unwrap();
    let s2 = store.create_session().unwrap();

    let sessions = store.list_sessions(10).unwrap();
    assert_eq!(sessions.len(), 2);
    // Most recent first
    assert_eq!(sessions[0].id, s2.id);
    assert_eq!(sessions[1].id, s1.id);
}

#[test]
fn save_and_load_messages() {
    let (_dir, store) = setup();
    let session = store.create_session().unwrap();

    store
        .save_message(&session.id, "user", Some("hello"), None, None)
        .unwrap();
    store
        .save_message(&session.id, "assistant", Some("hi there"), None, None)
        .unwrap();

    let messages = store.load_messages(&session.id).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content.as_deref(), Some("hello"));
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content.as_deref(), Some("hi there"));
}

#[test]
fn save_tool_call_entry() {
    let (_dir, store) = setup();
    let session = store.create_session().unwrap();
    let msg_id = store
        .save_message(&session.id, "assistant", None, Some("[]"), None)
        .unwrap();

    store
        .save_tool_call(&ToolCallEntry {
            session_id: &session.id,
            message_id: &msg_id,
            tool_name: "file_read",
            arguments: r#"{"path":"test.txt"}"#,
            result: Some("file contents"),
            duration_ms: Some(42),
            permission: "allowed",
        })
        .unwrap();
}

#[test]
fn list_sessions_respects_limit() {
    let (_dir, store) = setup();
    for _ in 0..5 {
        store.create_session().unwrap();
    }
    let sessions = store.list_sessions(3).unwrap();
    assert_eq!(sessions.len(), 3);
}

#[test]
fn empty_session_list() {
    let (_dir, store) = setup();
    let sessions = store.list_sessions(10).unwrap();
    assert!(sessions.is_empty());
}

#[test]
fn find_latest_resumable() {
    let (_dir, store) = setup();
    let s1 = store.create_session().unwrap();
    let s2 = store.create_session().unwrap();

    // Both are active, s2 is most recent
    let found = store.find_latest_resumable().unwrap().unwrap();
    assert_eq!(found.id, s2.id);

    // Mark s2 as completed — s1 should be found
    store
        .update_session_status(&s2.id, &SessionStatus::Completed)
        .unwrap();
    let found = store.find_latest_resumable().unwrap().unwrap();
    assert_eq!(found.id, s1.id);

    // Mark s1 as paused — still resumable
    store
        .update_session_status(&s1.id, &SessionStatus::Paused)
        .unwrap();
    let found = store.find_latest_resumable().unwrap().unwrap();
    assert_eq!(found.id, s1.id);

    // Mark s1 as completed — nothing resumable
    store
        .update_session_status(&s1.id, &SessionStatus::Completed)
        .unwrap();
    assert!(store.find_latest_resumable().unwrap().is_none());
}

#[test]
fn find_by_prefix() {
    let (_dir, store) = setup();
    let s1 = store.create_session().unwrap();

    let prefix = &s1.id[..8];
    let found = store.find_by_prefix(prefix).unwrap().unwrap();
    assert_eq!(found.id, s1.id);

    // Non-matching prefix
    assert!(store.find_by_prefix("zzzzzzz").unwrap().is_none());
}

#[test]
fn session_status_transitions() {
    let (_dir, store) = setup();
    let session = store.create_session().unwrap();
    assert_eq!(session.status, SessionStatus::Active);

    store
        .update_session_status(&session.id, &SessionStatus::Paused)
        .unwrap();
    let sessions = store.list_sessions(1).unwrap();
    assert_eq!(sessions[0].status, SessionStatus::Paused);

    store
        .update_session_status(&session.id, &SessionStatus::Active)
        .unwrap();
    let sessions = store.list_sessions(1).unwrap();
    assert_eq!(sessions[0].status, SessionStatus::Active);

    store
        .update_session_status(&session.id, &SessionStatus::Completed)
        .unwrap();
    let sessions = store.list_sessions(1).unwrap();
    assert_eq!(sessions[0].status, SessionStatus::Completed);
}

#[test]
fn search_sessions_finds_matching_content() {
    let (_dir, store) = setup();
    let s1 = store.create_session().unwrap();

    store
        .save_message(&s1.id, "user", Some("how do I configure docker"), None, None)
        .unwrap();
    store
        .save_message(
            &s1.id,
            "assistant",
            Some("You can use a Dockerfile"),
            None,
            None,
        )
        .unwrap();

    let results = store.search_sessions("docker", 10).unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].session_id, s1.id);
}

#[test]
fn search_sessions_no_results() {
    let (_dir, store) = setup();
    let s1 = store.create_session().unwrap();

    store
        .save_message(&s1.id, "user", Some("hello world"), None, None)
        .unwrap();

    let results = store.search_sessions("kubernetes", 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_sessions_across_multiple_sessions() {
    let (_dir, store) = setup();
    let s1 = store.create_session().unwrap();
    let s2 = store.create_session().unwrap();

    store
        .save_message(&s1.id, "user", Some("deploy to production"), None, None)
        .unwrap();
    store
        .save_message(&s2.id, "user", Some("deploy to staging"), None, None)
        .unwrap();

    let results = store.search_sessions("deploy", 10).unwrap();
    assert_eq!(results.len(), 2);
}
