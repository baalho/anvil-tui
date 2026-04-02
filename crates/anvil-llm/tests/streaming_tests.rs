//! Integration tests for the LLM client's SSE streaming and retry logic.
//! Uses wiremock to mock the OpenAI-compatible chat completions endpoint.

use anvil_config::ProviderConfig;
use anvil_llm::LlmClient;
use anvil_llm::StreamEvent;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build SSE response body from a list of JSON data chunks.
/// Each chunk is wrapped in `data: ...\n\n` framing, with `data: [DONE]` at the end.
fn sse_body(chunks: &[&str]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

/// Create a content delta chunk (assistant text).
fn content_chunk(content: &str) -> String {
    serde_json::json!({
        "choices": [{
            "delta": { "content": content },
            "finish_reason": null
        }]
    })
    .to_string()
}

/// Create a tool call delta chunk.
fn tool_call_chunk(index: usize, id: Option<&str>, name: Option<&str>, args: &str) -> String {
    let mut function = serde_json::Map::new();
    if let Some(n) = name {
        function.insert("name".into(), serde_json::json!(n));
    }
    function.insert("arguments".into(), serde_json::json!(args));

    serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": index,
                    "id": id,
                    "function": function
                }]
            },
            "finish_reason": null
        }]
    })
    .to_string()
}

/// Create a usage chunk (final stats).
fn usage_chunk(prompt: u64, completion: u64) -> String {
    serde_json::json!({
        "choices": [{
            "delta": {},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "total_tokens": prompt + completion
        }
    })
    .to_string()
}

async fn setup() -> (MockServer, LlmClient) {
    let server = MockServer::start().await;
    let config = ProviderConfig {
        base_url: server.uri(),
        model: "test-model".to_string(),
        ..Default::default()
    };
    let client = LlmClient::new(config).unwrap();
    (server, client)
}

fn empty_request() -> anvil_llm::ChatRequest {
    anvil_llm::ChatRequest {
        model: String::new(),
        messages: vec![anvil_llm::ChatMessage {
            role: anvil_llm::Role::User,
            content: Some("hello".to_string()),
            tool_calls: None,
            tool_call_id: None,
        }],
        stream: false,
        tools: None,
        tool_choice: None,
        temperature: None,
        top_p: None,
        min_p: None,
        repeat_penalty: None,
        top_k: None,
    }
}

#[tokio::test]
async fn stream_content_deltas() {
    let (server, mut client) = setup().await;
    let c1 = content_chunk("Hello");
    let c2 = content_chunk(" world");
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

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let mut rx = client
        .chat_stream(&mut request, cancel, |_, _, _| {})
        .await
        .unwrap();

    let mut content = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(delta) => content.push_str(&delta),
            StreamEvent::Done => break,
            _ => {}
        }
    }
    assert_eq!(content, "Hello world");
}

#[tokio::test]
async fn stream_tool_call_assembly() {
    let (server, mut client) = setup().await;
    // Tool call arrives in 3 chunks: id+name, args part 1, args part 2
    let c1 = tool_call_chunk(0, Some("call_1"), Some("file_read"), "{\"path\":");
    let c2 = tool_call_chunk(0, None, None, " \"src/");
    let c3 = tool_call_chunk(0, None, None, "main.rs\"}");
    let body = sse_body(&[&c1, &c2, &c3]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let mut rx = client
        .chat_stream(&mut request, cancel, |_, _, _| {})
        .await
        .unwrap();

    let mut deltas = Vec::new();
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => {
                deltas.push((index, id, name, arguments_delta));
            }
            StreamEvent::Done => break,
            _ => {}
        }
    }

    assert_eq!(deltas.len(), 3);
    // First chunk has id and name
    assert_eq!(deltas[0].1, Some("call_1".to_string()));
    assert_eq!(deltas[0].2, Some("file_read".to_string()));
    // All chunks contribute to arguments
    let full_args: String = deltas.iter().map(|d| d.3.as_str()).collect();
    assert_eq!(full_args, "{\"path\": \"src/main.rs\"}");
}

#[tokio::test]
async fn stream_usage_stats() {
    let (server, mut client) = setup().await;
    let c1 = content_chunk("hi");
    let c2 = usage_chunk(100, 5);
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

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let mut rx = client
        .chat_stream(&mut request, cancel, |_, _, _| {})
        .await
        .unwrap();

    let mut got_usage = false;
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Usage(u) => {
                assert_eq!(u.prompt_tokens, 100);
                assert_eq!(u.completion_tokens, 5);
                got_usage = true;
            }
            StreamEvent::Done => break,
            _ => {}
        }
    }
    assert!(got_usage, "expected Usage event");
}

#[tokio::test]
async fn stream_cancellation() {
    let (server, mut client) = setup().await;
    // Long stream — but we cancel after first chunk
    let mut chunks = Vec::new();
    for i in 0..100 {
        chunks.push(content_chunk(&format!("chunk{i} ")));
    }
    let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
    let body = sse_body(&chunk_refs);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();
    let mut rx = client
        .chat_stream(&mut request, cancel, |_, _, _| {})
        .await
        .unwrap();

    let mut count = 0;
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(_) => {
                count += 1;
                if count >= 2 {
                    cancel_clone.cancel();
                }
            }
            StreamEvent::Done => break,
            _ => {}
        }
    }
    // Should have received some content but not all 100 chunks
    assert!(count >= 2, "got at least 2 chunks before cancel");
    assert!(count < 100, "cancelled before all 100 chunks");
}

#[tokio::test]
async fn stream_error_on_disconnect() {
    let (server, mut client) = setup().await;
    // Return a truncated SSE body (no [DONE], connection drops)
    let body = format!("data: {}\n\n", content_chunk("partial"));
    // Don't add [DONE] — the stream will end abruptly

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let mut rx = client
        .chat_stream(&mut request, cancel, |_, _, _| {})
        .await
        .unwrap();

    let mut got_content = false;
    let mut got_done = false;
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(d) => {
                assert_eq!(d, "partial");
                got_content = true;
            }
            StreamEvent::Done => {
                got_done = true;
                break;
            }
            _ => {}
        }
    }
    assert!(got_content, "should have received partial content");
    assert!(got_done, "should have received Done after stream ends");
}

#[tokio::test]
async fn permanent_error_no_retry() {
    let (server, mut client) = setup().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_string("model not found"))
        .expect(1) // Should only be called once (no retry)
        .mount(&server)
        .await;

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let result = client.chat_stream(&mut request, cancel, |_, _, _| {}).await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("404"), "got: {err}");
}

#[tokio::test]
async fn retry_on_server_error() {
    let (server, mut client) = setup().await;
    let c1 = content_chunk("recovered");
    let body = sse_body(&[&c1]);

    // First call returns 503, second succeeds
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let mut rx = client
        .chat_stream(&mut request, cancel, move |_, _, _| {})
        .await
        .unwrap();

    let mut content = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::ContentDelta(d) => content.push_str(&d),
            StreamEvent::Done => break,
            _ => {}
        }
    }
    assert_eq!(content, "recovered");
}

#[tokio::test]
async fn empty_stream_produces_done() {
    let (server, mut client) = setup().await;
    let body = sse_body(&[]);

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(body)
                .append_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let mut request = empty_request();
    let cancel = CancellationToken::new();
    let mut rx = client
        .chat_stream(&mut request, cancel, |_, _, _| {})
        .await
        .unwrap();

    let mut got_done = false;
    while let Some(event) = rx.recv().await {
        if matches!(event, StreamEvent::Done) {
            got_done = true;
            break;
        }
    }
    assert!(got_done);
}
