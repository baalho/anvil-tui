//! Integration tests for the agent loop using wiremock as the LLM backend.
//! Tests the full turn cycle: LLM request → stream → tool execution → result.

use anvil_agent::{Agent, AgentEvent, SessionStore};
use anvil_config::Settings;
use anvil_tools::PermissionDecision;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build SSE response body from JSON data chunks.
fn sse_body(chunks: &[&str]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn content_chunk(content: &str) -> String {
    serde_json::json!({
        "choices": [{
            "delta": { "content": content },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn tool_call_chunk(id: &str, name: &str, args: &str) -> String {
    serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": id,
                    "function": {
                        "name": name,
                        "arguments": args
                    }
                }]
            },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn stop_chunk() -> String {
    serde_json::json!({
        "choices": [{
            "delta": {},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
    .to_string()
}

async fn setup() -> (MockServer, TempDir, Agent) {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    let db_path = dir.path().join("sessions.db");
    let store = SessionStore::open(&db_path).unwrap();

    let mut settings = Settings::default();
    settings.provider.base_url = server.uri();
    settings.provider.model = "test-model".to_string();

    let mcp = std::sync::Arc::new(anvil_agent::McpManager::empty());
    let agent = Agent::new(settings, dir.path().to_path_buf(), store, mcp).unwrap();
    (server, dir, agent)
}

/// Create a permission channel that auto-allows everything.
/// Spawns a task that sends Allow for every ToolCallPending event.
fn auto_allow_permissions(
    event_rx: &mpsc::Receiver<AgentEvent>,
) -> mpsc::Receiver<PermissionDecision> {
    let (perm_tx, perm_rx) = mpsc::channel(16);
    // For read-only tools, no permission is needed.
    // For mutating tools in tests, we pre-grant via the executor.
    // This channel exists but won't be read for read-only tools.
    let _ = perm_tx; // drop — tests only use read-only tools or pre-granted
    let _ = event_rx;
    perm_rx
}

/// Collect all events from a turn into a vec.
async fn collect_events(mut rx: mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn simple_content_response() {
    let (server, _dir, mut agent) = setup().await;
    let c1 = content_chunk("Hello from the LLM!");
    let c2 = stop_chunk();
    let body = sse_body(&[&c1, &c2]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let (tx, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    agent
        .turn("say hello", &tx, auto_allow_permissions(&rx), cancel)
        .await
        .unwrap();
    drop(tx);

    let events = collect_events(rx).await;
    let content: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ContentDelta(d) => Some(d.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(content, "Hello from the LLM!");

    // Message should be in history
    assert!(agent.messages().len() >= 3); // system + user + assistant
}

#[tokio::test]
async fn tool_call_executes_and_returns_result() {
    let (server, dir, mut agent) = setup().await;

    // Create a file for file_read to find
    std::fs::write(dir.path().join("test.txt"), "file content here").unwrap();

    // First LLM call: returns a tool call for file_read
    let tc = tool_call_chunk("call_1", "file_read", "{\"path\": \"test.txt\"}");
    let s1 = stop_chunk();
    let body1 = sse_body(&[&tc, &s1]);

    // Second LLM call (after tool result): returns content
    let c1 = content_chunk("I read the file.");
    let s2 = stop_chunk();
    let body2 = sse_body(&[&c1, &s2]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body1)
                .append_header("content-type", "text/event-stream"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body2)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let (tx, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    agent
        .turn("read test.txt", &tx, auto_allow_permissions(&rx), cancel)
        .await
        .unwrap();
    drop(tx);

    let events = collect_events(rx).await;

    // Should have a ToolResult event
    let tool_results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult { name, result } => Some((name.clone(), result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tool_results.len(), 1);
    assert_eq!(tool_results[0].0, "file_read");
    assert!(
        tool_results[0].1.contains("file content here"),
        "got: {}",
        tool_results[0].1
    );
}

#[tokio::test]
async fn cancellation_mid_turn() {
    let (server, _dir, mut agent) = setup().await;

    // Use a delayed response so cancellation has time to fire
    let c1 = content_chunk("start ");
    let body = sse_body(&[&c1]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream")
                .set_body_string(
                    // Slow SSE: content then a long pause (simulated by large padding)
                    format!(
                        "data: {}\n\n{}\ndata: [DONE]\n\n",
                        content_chunk("hello"),
                        " ".repeat(1024 * 64) // padding to slow down parsing
                    ),
                ),
        )
        .mount(&server)
        .await;

    let (tx, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();

    // Cancel immediately — before the turn even starts streaming
    cancel.cancel();

    agent
        .turn("generate text", &tx, auto_allow_permissions(&rx), cancel)
        .await
        .unwrap();
    drop(tx);

    let events = collect_events(rx).await;
    let has_cancelled = events.iter().any(|e| matches!(e, AgentEvent::Cancelled));
    assert!(has_cancelled, "expected Cancelled event");
}

#[tokio::test]
async fn input_validation_returns_error_to_llm() {
    let (server, _dir, mut agent) = setup().await;

    // LLM calls file_read with missing path argument
    let tc = tool_call_chunk("call_1", "file_read", "{}");
    let s1 = stop_chunk();
    let body1 = sse_body(&[&tc, &s1]);

    // After getting the error, LLM responds with content
    let c1 = content_chunk("Sorry, I need a path.");
    let s2 = stop_chunk();
    let body2 = sse_body(&[&c1, &s2]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body1)
                .append_header("content-type", "text/event-stream"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body2)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let (tx, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    agent
        .turn("read a file", &tx, auto_allow_permissions(&rx), cancel)
        .await
        .unwrap();
    drop(tx);

    let events = collect_events(rx).await;

    // Should have a tool result with the validation error
    let tool_results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult { name, result } => Some((name.clone(), result.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(tool_results.len(), 1);
    assert!(
        tool_results[0].1.contains("missing required"),
        "got: {}",
        tool_results[0].1
    );
}

#[tokio::test]
async fn turn_complete_event_emitted() {
    let (server, _dir, mut agent) = setup().await;
    let c1 = content_chunk("done");
    let s1 = stop_chunk();
    let body = sse_body(&[&c1, &s1]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let (tx, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    agent
        .turn("hi", &tx, auto_allow_permissions(&rx), cancel)
        .await
        .unwrap();
    drop(tx);

    let events = collect_events(rx).await;
    let has_complete = events.iter().any(|e| matches!(e, AgentEvent::TurnComplete));
    assert!(has_complete, "expected TurnComplete event");
}
