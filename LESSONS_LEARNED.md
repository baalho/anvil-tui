# Lessons Learned

After-action review from building Anvil — what worked, what didn't,
and patterns to carry forward.

## What Worked

### Shell commands as strings, not argv arrays
LLMs generate `ls -la /tmp`, not `["ls", "-la", "/tmp"]`. Accepting string
commands via `sh -c` eliminated 100% of shell tool failures. This was the
single highest-impact fix in the entire project.

### Readline over TUI
The ratatui TUI blocked during LLM generation and had race conditions with
streaming output. Replacing it with a simple readline-style interface
(crossterm for colors, raw mode for single-keypress prompts) was simpler
and more reliable. Lesson: don't build UI complexity you don't need.

### Option<Agent> pattern for async ownership
Rust's ownership rules prevent moving `&mut self` into a spawned task.
The `Option<Agent>` take/put pattern solves this cleanly:
```rust
let agent = self.agent.take().unwrap();
let handle = tokio::spawn(async move { agent.turn(...).await });
// ... process events ...
self.agent = Some(handle.await??);
```

### Tail-truncation with temp file fallback
When tool output exceeds limits, keeping the tail (most recent output)
is more useful than the head. Saving the full output to a temp file
lets the LLM access it if needed.

### Retry with Retryable/Permanent distinction
Not all errors should be retried. 404 is permanent (model not found).
429 is retryable (rate limit). The `RetryError` enum makes this explicit
and prevents wasting time retrying unrecoverable errors.

### Model profiles as TOML files
Different models need different sampling params. Hardcoding these would
require rebuilding Anvil for each new model. TOML files in `.anvil/models/`
are user-editable and don't require recompilation.

### Skills as dual-purpose documents
Making each skill file serve as both prompt template AND documentation
means the docs are always in sync with the actual instructions. Users
read the same content the LLM reads.

## What Didn't Work

### ratatui TUI
Built a full terminal UI with panels, scrolling, and status bars. It
blocked during LLM streaming, had race conditions, and was 3x the code
of the readline replacement. Deleted entirely.

### Shell tool with argv arrays
The original shell tool expected `["ls", "-la"]` format. Every model
generated string commands instead. 100% failure rate. Fixed by accepting
strings and running via `sh -c`.

### Retrying 404 errors
Before the Retryable/Permanent distinction, all HTTP errors were retried.
A 404 (model not found) would retry 3 times with exponential backoff,
wasting 15+ seconds before failing. Now it fails immediately.

### Default model assumption
Assuming a specific model would be installed. Users had different models.
Fixed with `auto_detect_model()` that queries the backend for available models
and falls back to the first available one.

### Ollama for all models
Unsloth explicitly warns against using GLM-4.7-Flash with Ollama due to
chat template conversion bugs. The multi-backend approach (Ollama, llama-server,
MLX) lets users pick the right backend per model.

## Patterns to Reuse

### Backend-agnostic client
All backends (Ollama, llama-server, MLX) expose the same OpenAI-compatible
API. Write one client, configure the URL. Backend-specific logic is limited
to model discovery endpoints.

### Frontmatter for metadata
YAML frontmatter in markdown files is a well-established pattern (Jekyll,
Hugo, Obsidian). It adds structured metadata without breaking the document
as plain markdown.

### Verification-based testing
Skills with `verify` commands are self-testing. `/skill verify docker`
runs `docker info` and reports pass/fail. This pattern scales to any
prerequisite check.

### The Ralph Loop
Autonomous retry-until-done with verification is powerful for tasks with
clear success criteria. The key insight: feed failure output back as context
for the next attempt. The LLM learns from its mistakes within the session.

## Anti-Patterns to Avoid

### Don't build UI before the core works
The TUI was built before the agent loop was stable. Every agent change
broke the UI. Build the core, test with CLI, add UI last.

### Don't assume model capabilities
Small models (7B) may ignore complex instructions. Keep tool definitions
simple. Use explicit step-by-step instructions in skills. Test with the
smallest model you plan to support.

### Don't env_clear() without an escape hatch
Clearing the environment is secure but breaks legitimate use cases.
Per-skill env declarations provide the escape hatch without opening
the floodgates.

### Don't retry everything
Distinguish between transient errors (network timeout, rate limit) and
permanent errors (auth failure, model not found). Retrying permanent
errors wastes time and confuses users.

## The Ralph Loop Methodology

Named after the "Ralph Wiggum loop" — an autonomous AI coding methodology:

1. Define a clear, verifiable success condition (`cargo test`, `docker compose ps`)
2. Give the agent a task and the verification command
3. Let it run: attempt → verify → learn from failure → retry
4. Hard limits prevent runaway: iterations, tokens, wall-clock time
5. The agent improves each iteration because failure output provides context

This works best when:
- Success is binary and machine-checkable (tests pass, service is healthy)
- The task is bounded (fix tests, not "make the code better")
- The model is capable enough to learn from error messages
- Limits are set conservatively (start with 5 iterations, increase if needed)
