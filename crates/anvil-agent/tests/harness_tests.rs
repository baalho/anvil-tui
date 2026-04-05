//! Integration tests for the multi-agent harness using wiremock.
//!
//! These tests exercise the full planner → generator → evaluator flow
//! with mock LLM responses. The mock server simulates an OpenAI-compatible
//! API that returns canned SSE responses for each agent phase.

use anvil_agent::harness::{self, HarnessEvent, HarnessState, HarnessStatus};
use anvil_config::Settings;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

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

fn stop_chunk() -> String {
    serde_json::json!({
        "choices": [{
            "delta": {},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150
        }
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

/// A responder that cycles through a list of response bodies.
/// Each call to respond() returns the next body in the list.
struct CyclingResponder {
    bodies: Vec<String>,
    counter: std::sync::atomic::AtomicUsize,
}

impl CyclingResponder {
    fn new(bodies: Vec<String>) -> Self {
        Self {
            bodies,
            counter: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl Respond for CyclingResponder {
    fn respond(&self, _request: &wiremock::Request) -> ResponseTemplate {
        let idx = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let body_idx = idx % self.bodies.len();
        ResponseTemplate::new(200)
            .set_body_string(self.bodies[body_idx].clone())
            .append_header("content-type", "text/event-stream")
    }
}

fn make_settings(server_url: &str) -> Settings {
    let mut settings = Settings::default();
    settings.provider.base_url = server_url.to_string();
    settings.provider.model = "test-model".to_string();
    settings.harness.max_sprints = 5;
    settings.harness.max_retries_per_sprint = 2;
    settings.harness.sprint_turn_limit = 3;
    settings.harness.max_total_tokens = 100_000;
    settings.harness.max_duration_minutes = 60;
    settings
}

/// Planner response: a structured plan with 2 sprints.
fn planner_response() -> String {
    let plan = r#"# Plan: add tests

## Sprint 1: Add unit tests for parser
- **Files:** src/parser.rs
- **Criteria:**
  - [ ] Parser has unit tests
  - [ ] Tests cover edge cases
- **Verify:** `echo "tests pass"`

## Sprint 2: Add integration tests
- **Files:** tests/integration.rs
- **Criteria:**
  - [ ] Integration test exists
- **Verify:** `echo "integration pass"`"#;

    let c = content_chunk(plan);
    let s = stop_chunk();
    sse_body(&[&c, &s])
}

/// Generator response: claims to have done the work + SPRINT:DONE marker.
fn generator_done_response() -> String {
    let content = "I've implemented the changes.\n\n## Handoff\n- Added tests\n- All passing\n\n[SPRINT:DONE]";
    let c = content_chunk(content);
    let s = stop_chunk();
    sse_body(&[&c, &s])
}

/// Generator response that uses a shell tool call (used in retry tests).
#[allow(dead_code)]
fn generator_tool_response() -> String {
    let tc = tool_call_chunk("call_1", "shell", r#"{"command": "echo hello"}"#);
    let s = stop_chunk();
    sse_body(&[&tc, &s])
}

/// Evaluator response: PASS verdict.
fn evaluator_pass_response() -> String {
    let content = r#"## Verdict: PASS

## Criteria
- [x] Parser has unit tests
- [x] Tests cover edge cases

## Verify: `echo "tests pass"`
Exit code: 0

## Feedback
All criteria met."#;

    // Evaluator will first try to run the verify command via shell tool
    let tc = tool_call_chunk(
        "eval_1",
        "shell",
        r#"{"command": "echo \"tests pass\""}"#,
    );
    let s1 = stop_chunk();
    let body1 = sse_body(&[&tc, &s1]);

    // Then it will produce the verdict
    let c = content_chunk(content);
    let s2 = stop_chunk();
    let body2 = sse_body(&[&c, &s2]);

    // Return just the verdict for simplicity — the evaluator may or may not
    // call tools depending on the prompt. We'll use the cycling responder.
    let _ = body1;
    body2
}

/// Evaluator response: FAIL verdict.
fn evaluator_fail_response() -> String {
    let content = r#"## Verdict: FAIL

## Criteria
- [x] Parser has unit tests
- [ ] Tests cover edge cases — no edge case tests found

## Feedback
Add tests for empty input and malformed data."#;

    let c = content_chunk(content);
    let s = stop_chunk();
    sse_body(&[&c, &s])
}

#[tokio::test]
async fn harness_plan_parsing_integration() {
    // Test that the planner's output gets correctly parsed into sprints
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    // Create a source file so repo map has something to scan
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/parser.rs"), "pub fn parse() {}").unwrap();

    // Mock: planner gets one call, then generator and evaluator get calls
    // For this test we just care about the plan being parsed
    let responses = vec![
        planner_response(),       // planner
        generator_done_response(), // generator sprint 1
        evaluator_pass_response(), // evaluator sprint 1
        generator_done_response(), // generator sprint 2
        evaluator_pass_response(), // evaluator sprint 2
    ];

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(responses))
        .mount(&server)
        .await;

    let settings = make_settings(&server.uri());
    let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    let workspace = dir.path().to_path_buf();
    let result = harness::run_harness(
        settings,
        workspace.clone(),
        "add tests",
        "echo ok",
        event_tx,
        cancel,
    )
    .await;

    assert!(result.is_ok(), "harness failed: {:?}", result.err());

    // Collect events
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Verify plan was generated
    let plan_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, HarnessEvent::PlanGenerated { .. }))
        .collect();
    assert_eq!(plan_events.len(), 1);
    if let HarnessEvent::PlanGenerated { sprints } = plan_events[0] {
        assert_eq!(*sprints, 2);
    }

    // Verify plan.md was written
    let hdir = harness::harness_dir(&workspace);
    let plan_content = std::fs::read_to_string(hdir.join("plan.md")).unwrap();
    assert!(plan_content.contains("Sprint 1"));
    assert!(plan_content.contains("Sprint 2"));

    // Verify state.toml shows completed
    let state = HarnessState::load(&hdir).unwrap();
    assert_eq!(state.harness.status, HarnessStatus::Completed);
    assert!(state.usage.total_tokens > 0);
}

#[tokio::test]
async fn harness_cancellation() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    // Planner response only — cancel before generator runs
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(vec![planner_response()]))
        .mount(&server)
        .await;

    let settings = make_settings(&server.uri());
    let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    // Cancel immediately after planner
    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        // Wait a bit for planner to finish, then cancel
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        cancel_clone.cancel();
    });

    let workspace = dir.path().to_path_buf();
    let result = harness::run_harness(
        settings,
        workspace.clone(),
        "add tests",
        "echo ok",
        event_tx,
        cancel,
    )
    .await;

    assert!(result.is_ok());

    // Drain events
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should have generated a plan but may not have completed all sprints
    let _plan_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, HarnessEvent::PlanGenerated { .. }))
        .collect();
    // Plan may or may not have been generated depending on timing
    // The key assertion is that it didn't panic or hang
}

#[tokio::test]
async fn harness_state_persisted_on_completion() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    // Single-sprint plan for simplicity
    let single_sprint_plan = {
        let plan = "## Sprint 1: Do the thing\nDo it.\n- [ ] It's done\n";
        let c = content_chunk(plan);
        let s = stop_chunk();
        sse_body(&[&c, &s])
    };

    let responses = vec![
        single_sprint_plan,        // planner
        generator_done_response(), // generator
        evaluator_pass_response(), // evaluator
    ];

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(responses))
        .mount(&server)
        .await;

    let settings = make_settings(&server.uri());
    let (event_tx, _event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    let workspace = dir.path().to_path_buf();
    harness::run_harness(
        settings,
        workspace.clone(),
        "do the thing",
        "echo ok",
        event_tx,
        cancel,
    )
    .await
    .unwrap();

    // Verify state
    let hdir = harness::harness_dir(&workspace);
    let state = HarnessState::load(&hdir).unwrap();
    assert_eq!(state.harness.status, HarnessStatus::Completed);
    assert_eq!(state.harness.prompt, "do the thing");
    assert_eq!(state.harness.verify_command, "echo ok");
    assert!(state.usage.planner_tokens > 0);
    assert!(state.usage.generator_tokens > 0);
    assert!(state.usage.evaluator_tokens > 0);
}

#[tokio::test]
async fn harness_artifacts_written() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    let responses = vec![
        planner_response(),        // planner
        generator_done_response(), // generator sprint 1
        evaluator_pass_response(), // evaluator sprint 1
        generator_done_response(), // generator sprint 2
        evaluator_pass_response(), // evaluator sprint 2
    ];

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(responses))
        .mount(&server)
        .await;

    let settings = make_settings(&server.uri());
    let (event_tx, _event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    let workspace = dir.path().to_path_buf();
    harness::run_harness(
        settings,
        workspace.clone(),
        "add tests",
        "echo ok",
        event_tx,
        cancel,
    )
    .await
    .unwrap();

    let hdir = harness::harness_dir(&workspace);

    // All artifacts should exist
    assert!(hdir.join("plan.md").exists(), "plan.md missing");
    assert!(hdir.join("state.toml").exists(), "state.toml missing");
    assert!(hdir.join("handoff.md").exists(), "handoff.md missing");
    assert!(hdir.join("eval.md").exists(), "eval.md missing");

    // Handoff should contain generator output
    let handoff = std::fs::read_to_string(hdir.join("handoff.md")).unwrap();
    assert!(!handoff.is_empty());

    // Eval should contain evaluator output
    let eval = std::fs::read_to_string(hdir.join("eval.md")).unwrap();
    assert!(eval.contains("PASS") || eval.contains("pass"));
}

#[tokio::test]
async fn harness_cleans_previous_run() {
    let dir = TempDir::new().unwrap();
    let hdir = harness::harness_dir(dir.path());

    // Create a stale harness directory
    std::fs::create_dir_all(&hdir).unwrap();
    std::fs::write(hdir.join("old_file.txt"), "stale data").unwrap();

    let server = MockServer::start().await;

    let responses = vec![
        {
            let c = content_chunk("## Sprint 1: Do it\nDo it.\n");
            let s = stop_chunk();
            sse_body(&[&c, &s])
        },
        generator_done_response(),
        evaluator_pass_response(),
    ];

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(responses))
        .mount(&server)
        .await;

    let settings = make_settings(&server.uri());
    let (event_tx, _event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    harness::run_harness(
        settings,
        dir.path().to_path_buf(),
        "test",
        "echo ok",
        event_tx,
        cancel,
    )
    .await
    .unwrap();

    // Old file should be gone
    assert!(!hdir.join("old_file.txt").exists());
    // New artifacts should exist
    assert!(hdir.join("state.toml").exists());
}

#[tokio::test]
async fn harness_retry_on_eval_fail() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    // Single sprint plan
    let single_sprint = {
        let plan = "## Sprint 1: Fix bug\nFix the bug.\n- [ ] Bug is fixed\n";
        let c = content_chunk(plan);
        let s = stop_chunk();
        sse_body(&[&c, &s])
    };

    let responses = vec![
        single_sprint,             // planner
        generator_done_response(), // generator attempt 1
        evaluator_fail_response(), // evaluator FAIL
        generator_done_response(), // generator attempt 2 (retry)
        evaluator_pass_response(), // evaluator PASS
    ];

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(responses))
        .mount(&server)
        .await;

    let settings = make_settings(&server.uri());
    let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    let workspace = dir.path().to_path_buf();
    harness::run_harness(
        settings,
        workspace.clone(),
        "fix bug",
        "echo ok",
        event_tx,
        cancel,
    )
    .await
    .unwrap();

    // Collect events
    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should have a FAIL then a retry then a PASS
    let eval_results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            HarnessEvent::SprintEvalResult {
                passed, attempt, ..
            } => Some((*passed, *attempt)),
            _ => None,
        })
        .collect();

    assert!(
        eval_results.len() >= 2,
        "expected at least 2 eval results, got {:?}",
        eval_results
    );
    // First eval should fail, second should pass
    assert!(!eval_results[0].0, "first eval should be FAIL");
    assert!(eval_results[1].0, "second eval should be PASS");

    // Should have a retry event
    let retries: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, HarnessEvent::SprintRetry { .. }))
        .collect();
    assert!(!retries.is_empty(), "expected retry event");

    // Final state should be completed
    let hdir = harness::harness_dir(&workspace);
    let state = HarnessState::load(&hdir).unwrap();
    assert_eq!(state.harness.status, HarnessStatus::Completed);
}

#[tokio::test]
async fn harness_fails_after_max_retries() {
    let server = MockServer::start().await;
    let dir = TempDir::new().unwrap();

    let single_sprint = {
        let plan = "## Sprint 1: Impossible task\nCan't be done.\n- [ ] Done\n";
        let c = content_chunk(plan);
        let s = stop_chunk();
        sse_body(&[&c, &s])
    };

    // All evaluator responses are FAIL — should exhaust retries
    let responses = vec![
        single_sprint,             // planner
        generator_done_response(), // generator attempt 1
        evaluator_fail_response(), // evaluator FAIL
        generator_done_response(), // generator attempt 2
        evaluator_fail_response(), // evaluator FAIL again
    ];

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(CyclingResponder::new(responses))
        .mount(&server)
        .await;

    let mut settings = make_settings(&server.uri());
    settings.harness.max_retries_per_sprint = 2; // only 2 attempts

    let (event_tx, mut event_rx) = mpsc::channel::<HarnessEvent>(64);
    let cancel = CancellationToken::new();

    let workspace = dir.path().to_path_buf();
    harness::run_harness(
        settings,
        workspace.clone(),
        "impossible",
        "echo ok",
        event_tx,
        cancel,
    )
    .await
    .unwrap();

    let mut events = Vec::new();
    while let Some(event) = event_rx.recv().await {
        events.push(event);
    }

    // Should have a HarnessFailed event
    let failed: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, HarnessEvent::HarnessFailed { .. }))
        .collect();
    assert!(!failed.is_empty(), "expected HarnessFailed event");

    // State should be Failed
    let hdir = harness::harness_dir(&workspace);
    let state = HarnessState::load(&hdir).unwrap();
    assert_eq!(state.harness.status, HarnessStatus::Failed);
}
