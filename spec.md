# Anvil v0.1.1 → v1.0 — Full Roadmap Spec

## Problem Statement

Anvil v0.1.0 is a working local-first coding agent with multi-backend support,
model profiles, skills, and autonomous mode. To become a daily-driver tool,
it needs: cancellation, thinking mode support, context management, interactive
autonomy, backend lifecycle, memory, model routing, error resilience,
extensibility, performance, cross-platform CI, community infrastructure,
and production stability guarantees.

This spec covers all 30 user stories from AGILE.md (v0.1.1 through v1.0),
plus dev environment setup and integration testing infrastructure.

## Current State

- 5 crates: `anvil-config`, `anvil-llm`, `anvil-tools`, `anvil-agent`, `anvil`
- 91 tests, 0 clippy warnings, 0 doc warnings
- 7 tools: shell, file_read, file_write, file_edit, grep, ls, find
- Multi-backend: Ollama, llama-server, MLX, Custom
- 5 model profiles, 14 bundled skills, autonomous mode (CLI only)
- Session persistence in SQLite, env passthrough per skill
- No Rust toolchain in current dev environment

---

## Milestone 0: Dev Environment Setup

### Requirements

**M0.1** — `.devcontainer/devcontainer.json` configures Rust 1.75+ toolchain,
cargo, clippy, rustfmt, and rust-analyzer.

**M0.2** — `automations.yaml` runs `cargo build` on environment start.

**M0.3** — `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo doc --no-deps` all pass after setup.

### Acceptance Criteria

- `cargo build` succeeds in the dev container
- `cargo test` runs all 91 existing tests
- rust-analyzer provides IDE support

---

## Milestone 1: v0.1.1 — Stability

### S1: Ctrl+C Cancellation

**Problem**: No way to cancel an in-flight LLM request without killing the process.

**Requirements**:
- S1.1 — `tokio_util::sync::CancellationToken` wired through `Agent::turn()` and `LlmClient::chat_stream()`
- S1.2 — Ctrl+C during streaming aborts the HTTP request and returns to prompt
- S1.3 — Ctrl+C during tool execution sends SIGTERM to the child process
- S1.4 — Double Ctrl+C within 1 second exits Anvil entirely
- S1.5 — Partial content received before cancellation is preserved in session

**Files**: `interactive.rs`, `agent.rs`, `client.rs`, `stream.rs`, `tools.rs`

**Tests**:
- Unit: CancellationToken propagation aborts stream parsing
- Unit: Double-press detection logic
- Integration: Mock SSE server, cancel mid-stream, verify partial content saved

### S2: Thinking Mode Parsing

**Problem**: Qwen3 and DeepSeek-R1 output `<think>` blocks that clutter output.

**Requirements**:
- S2.1 — Stream parser detects `<think>...</think>` blocks in content deltas
- S2.2 — Thinking content is stripped from displayed output by default
- S2.3 — Thinking content is stored in session history (for debugging)
- S2.4 — `/think` toggle command shows/hides thinking blocks for the session
- S2.5 — When visible, thinking blocks render in dim/grey color

**Files**: `stream.rs` (parser), `interactive.rs` (display), `commands.rs` (`/think`), `agent.rs` (storage)

**Tests**:
- Unit: Parse `<think>` blocks spanning multiple SSE chunks
- Unit: Nested or malformed `<think>` tags handled gracefully
- Unit: Toggle state persists across turns

### S3: Context Compaction

**Problem**: Long sessions exceed context window with no recovery path.

**Requirements**:
- S3.1 — `/clear` summarizes conversation by sending a compaction prompt to the LLM
- S3.2 — System prompt and most recent N messages (configurable, default 4) preserved
- S3.3 — Tool call history compressed to name+result summaries
- S3.4 — Token count drops after compaction (verified via estimate)
- S3.5 — Compaction event shown to user with before/after token counts

**Files**: `agent.rs` (compaction logic), `commands.rs` (`/clear`), `interactive.rs` (display)

**Tests**:
- Unit: Message list reduction preserves system prompt and recent messages
- Unit: Token estimate decreases after compaction
- Integration: Mock LLM returns summary, verify message list shrinks

### v0.1.1 Acceptance Criteria

1. Ctrl+C stops streaming and returns to prompt within 500ms
2. Double Ctrl+C exits cleanly
3. `<think>` blocks hidden by default, visible with `/think`
4. `/clear` reduces token count while preserving recent context
5. All 91 existing tests still pass
6. No new clippy warnings

---

## Milestone 2: v0.2.0 — Interactive Autonomy

### S4: Interactive Ralph Loop

**Requirements**:
- S4.1 — `/ralph <prompt> --verify <cmd>` starts autonomous loop from interactive mode
- S4.2 — Progress displayed: iteration count, verify result, elapsed time
- S4.3 — Ctrl+C (from S1) stops the loop and returns to prompt
- S4.4 — Results stored in current session history
- S4.5 — Optional `--max-iterations N` flag (default from config)

**Files**: `commands.rs`, `interactive.rs`, `agent.rs`

### S5: Backend Lifecycle Management

**Requirements**:
- S5.1 — `anvil --backend llama --model <path.gguf>` starts llama-server as child process
- S5.2 — Anvil stops llama-server on exit (SIGTERM, then SIGKILL after 5s)
- S5.3 — If llama-server already running on target port, connect to it
- S5.4 — Health check loop waits for backend readiness before first prompt
- S5.5 — `/backend start llama <model_path>` from interactive mode
- S5.6 — `/backend stop` kills managed backend process

**Files**: new `crates/anvil-config/src/backend_lifecycle.rs`, `main.rs`, `commands.rs`

### S6: Model Switching with Profile Reload

**Requirements**:
- S6.1 — `/model <name>` reloads matching profile and applies sampling params
- S6.2 — `/model` with no args shows current model + active profile info
- S6.3 — Switching to model with no profile clears sampling overrides
- S6.4 — Backend-specific model discovery (Ollama `/api/tags`, others `/v1/models`)

**Files**: `commands.rs`, `agent.rs`, `client.rs`

### v0.2.0 Acceptance Criteria

1. `/ralph "fix tests" --verify "cargo test"` runs loop inline
2. Ctrl+C stops Ralph Loop mid-iteration
3. `anvil --backend llama --model model.gguf` starts and manages llama-server
4. `/model devstral` applies Devstral sampling config automatically

---

## Milestone 3: v0.3.0 — Context Intelligence

### S7: Sliding Window Context

**Requirements**:
- S7.1 — When context exceeds 90% of window, auto-compact oldest messages
- S7.2 — System prompt and most recent N messages always preserved
- S7.3 — Notification shown when auto-compaction occurs
- S7.4 — Configurable threshold in `settings.toml` (`agent.auto_compact_threshold`)

**Files**: `agent.rs`, `settings.rs`

### S8: Session Search

**Requirements**:
- S8.1 — `anvil history --search "docker"` searches session content
- S8.2 — Search covers user messages, assistant responses, tool outputs
- S8.3 — Results show session ID, date, matching snippet
- S8.4 — SQLite FTS5 for efficient full-text search

**Files**: `session.rs`, `main.rs`

### S9: Project Memory

**Requirements**:
- S9.1 — `.anvil/memory/` stores learned patterns as markdown files
- S9.2 — Memory loaded into context on session start (appended to system prompt)
- S9.3 — `/memory` lists stored patterns
- S9.4 — `/memory add <pattern>` saves a new pattern
- S9.5 — `/memory clear` removes all patterns

**Files**: new `crates/anvil-agent/src/memory.rs`, `system_prompt.rs`, `commands.rs`

### v0.3.0 Acceptance Criteria

1. Auto-compaction triggers at 90% context usage without user action
2. `anvil history --search "docker"` returns matching sessions
3. `/memory add` persists across sessions

---

## Milestone 4: v0.4.0 — Multi-Model

### S10: Model Routing

**Requirements**:
- S10.1 — `/route shell qwen3:8b` routes shell tasks to a specific model
- S10.2 — Default model handles unrouted tasks
- S10.3 — Routing rules persist in `.anvil/config.toml`
- S10.4 — Route by tool name or skill category

**Files**: new `crates/anvil-agent/src/routing.rs`, `agent.rs`, `settings.rs`

### S11: Parallel Tool Execution

**Requirements**:
- S11.1 — When LLM returns multiple tool calls, execute concurrently if independent
- S11.2 — Results collected and sent back in original order
- S11.3 — Errors in one tool don't block others
- S11.4 — Read-only tools always parallelizable; mutating tools serialized

**Files**: `executor.rs`, `agent.rs`, `permission.rs`

### S12: Cost Tracking

**Requirements**:
- S12.1 — `/stats` shows estimated cost when `PricingConfig` is set
- S12.2 — Cost calculated from prompt + completion token counts
- S12.3 — Local models show $0.00
- S12.4 — Per-session and cumulative cost tracking

**Files**: `usage.rs`, `commands.rs`, `agent.rs`

### v0.4.0 Acceptance Criteria

1. `/route shell qwen3:8b` sends shell-related turns to smaller model
2. Multiple independent tool calls execute concurrently
3. `/stats` shows accurate cost for configured pricing

---

## Milestone 5: v0.5.0 — Resilience

### S13: Graceful Error Recovery

**Requirements**:
- S13.1 — Backend disconnection mid-stream triggers retry after reconnection
- S13.2 — Session state preserved during disconnection
- S13.3 — User notified of disconnection and recovery
- S13.4 — Configurable reconnection timeout and max retries

**Files**: `client.rs`, `retry.rs`, `stream.rs`

### S14: Tool Timeout Handling

**Requirements**:
- S14.1 — Shell commands exceeding `shell_timeout_secs` killed with SIGTERM
- S14.2 — If SIGTERM ineffective, SIGKILL after 5 seconds
- S14.3 — Timeout reported to LLM as error with partial output captured
- S14.4 — Configurable per-tool timeout overrides

**Files**: `tools.rs`, `executor.rs`

### S15: Input Validation

**Requirements**:
- S15.1 — Missing required tool arguments produce clear error messages
- S15.2 — Invalid JSON in tool call arguments handled gracefully
- S15.3 — Path traversal attempts (`../../../etc/passwd`) blocked
- S15.4 — Validation errors fed back to LLM for self-correction

**Files**: `executor.rs`, `tools.rs`, `definitions.rs`

### v0.5.0 Acceptance Criteria

1. Backend restart mid-session recovers without data loss
2. `sleep 999` killed after timeout with partial output
3. Malformed tool arguments produce actionable error messages

---

## Milestone 6: v0.6.0 — Extensibility

### S16: Custom Tool Plugins

**Requirements**:
- S16.1 — `.anvil/tools/*.toml` defines custom tools with name, description, params, shell template
- S16.2 — Custom tools appear in LLM's tool list alongside built-ins
- S16.3 — Same permission model as built-in tools
- S16.4 — Template variables: `{{arg_name}}` substitution from LLM arguments

**Files**: new `crates/anvil-tools/src/plugins.rs`, `executor.rs`, `definitions.rs`

### S17: Skill Composition

**Requirements**:
- S17.1 — `depends: [git-workflow, docker]` in frontmatter activates dependencies
- S17.2 — Circular dependencies detected and reported with error
- S17.3 — Transitive dependencies resolved (A depends B depends C → all three active)

**Files**: `skills.rs`, `agent.rs`

### S18: Hook System

**Requirements**:
- S18.1 — `.anvil/hooks/pre-shell.sh` runs before every shell command
- S18.2 — `.anvil/hooks/post-edit.sh` runs after every file edit
- S18.3 — Hook failure blocks tool execution (configurable via `hooks.block_on_failure`)
- S18.4 — Hook stdout/stderr captured and logged

**Files**: new `crates/anvil-tools/src/hooks.rs`, `executor.rs`, `tools.rs`

### v0.6.0 Acceptance Criteria

1. Custom tool in `.anvil/tools/deploy.toml` callable by LLM
2. Skill with `depends: [docker]` auto-activates docker skill
3. `pre-shell.sh` hook blocks dangerous commands

---

## Milestone 7: v0.7.0 — Performance

### S19: Incremental Context Loading

**Requirements**:
- S19.1 — File contents loaded on-demand via tool calls, not upfront
- S19.2 — Previously read files cached in session (avoid re-reading unchanged files)
- S19.3 — Cache invalidated when file_write or file_edit modifies a cached file

**Files**: `agent.rs`, `executor.rs`

### S20: Streaming Tool Output

**Requirements**:
- S20.1 — Shell command stdout streams to terminal in real-time
- S20.2 — LLM receives final output after command completes
- S20.3 — Long-running commands show progress indicator

**Files**: `tools.rs`, `interactive.rs`

### S21: Binary Size Optimization

**Requirements**:
- S21.1 — Release profile: `lto = true`, `strip = true`, `codegen-units = 1`
- S21.2 — Target binary under 10MB (currently 13MB)
- S21.3 — Feature flags for optional components (e.g., `sqlite` for sessions)

**Files**: `Cargo.toml` (workspace), release profile

### v0.7.0 Acceptance Criteria

1. File cache avoids redundant reads (measurable via tool call count)
2. `cargo build` output streams to terminal during execution
3. Release binary under 10MB

---

## Milestone 8: v0.8.0 — Cross-Platform

### S22: Linux CI

**Requirements**:
- S22.1 — GitHub Actions workflow: `cargo test` on Ubuntu latest
- S22.2 — Clippy and doc warnings fail the build
- S22.3 — Runs on push to master and on PRs

**Files**: `.github/workflows/ci.yml`

### S23: Windows/WSL Testing

**Requirements**:
- S23.1 — `cargo build` succeeds on Windows (WSL)
- S23.2 — Shell tool uses `sh -c` on WSL, `cmd.exe /C` on native Windows
- S23.3 — CI job for Windows/WSL (optional, can be manual)

**Files**: `tools.rs` (platform detection), `.github/workflows/ci.yml`

### S24: macOS ARM CI

**Requirements**:
- S24.1 — GitHub Actions: `cargo test` on `macos-14` (ARM)
- S24.2 — Release artifacts include macOS ARM binary
- S24.3 — Release workflow triggered by git tags

**Files**: `.github/workflows/ci.yml`, `.github/workflows/release.yml`

### v0.8.0 Acceptance Criteria

1. CI green on Linux, macOS ARM, and Windows/WSL
2. Release workflow produces binaries for all platforms

---

## Milestone 9: v0.9.0 — Community

### S25: Contributing Guide

**Requirements**:
- S25.1 — `CONTRIBUTING.md`: build setup, test commands, PR process
- S25.2 — Issue templates for bugs and feature requests
- S25.3 — PR template with checklist

**Files**: `CONTRIBUTING.md`, `.github/ISSUE_TEMPLATE/`, `.github/pull_request_template.md`

### S26: Changelog

**Requirements**:
- S26.1 — `CHANGELOG.md` in Keep a Changelog format
- S26.2 — Entries for all versions from v0.1.0 onward
- S26.3 — Updated with every version bump

**Files**: `CHANGELOG.md`

### S27: Install Script

**Requirements**:
- S27.1 — `install.sh`: detects OS/arch, downloads binary, installs to `~/.local/bin/`
- S27.2 — Checksum verification (SHA256)
- S27.3 — Works on macOS and Linux

**Files**: `scripts/install.sh`

### v0.9.0 Acceptance Criteria

1. New contributor: zero to running tests in under 5 minutes
2. CHANGELOG covers all versions
3. Install script works on macOS and Ubuntu

---

## Milestone 10: v1.0.0 — Daily Driver

### S28: Stability Guarantee

**Requirements**:
- S28.1 — 8-hour soak test with continuous prompts passes
- S28.2 — Memory usage stable (no unbounded growth)
- S28.3 — All error paths tested

**Files**: `tests/soak_test.rs`, memory profiling

### S29: Configuration Migration

**Requirements**:
- S29.1 — Config format changes include migration logic
- S29.2 — Old `.anvil/config.toml` upgraded in place
- S29.3 — Migration logged so users know what changed

**Files**: new `crates/anvil-config/src/migration.rs`, `lib.rs`

### S30: Offline Documentation

**Requirements**:
- S30.1 — `anvil help tools` lists all tools with descriptions
- S30.2 — `anvil help skills` explains the skills system
- S30.3 — `anvil help config` shows all configuration options
- S30.4 — Content compiled into binary (no external files needed)

**Files**: `main.rs` (help subcommand), embedded content

### v1.0.0 Acceptance Criteria

1. 8-hour soak test passes
2. Config migration from v0.1.0 format works
3. `anvil help <topic>` works offline for all topics
4. No known crashes or data loss bugs

---

## Integration Testing Infrastructure

Applies across all milestones. Set up in M0, expanded per milestone.

**Requirements**:
- T1 — Add `wiremock` as dev dependency for mock HTTP server
- T2 — Mock SSE streaming endpoint for `/v1/chat/completions`
- T3 — Test fixtures: multi-turn conversations with tool calls
- T4 — Cancellation tests: abort mid-stream, verify partial state
- T5 — Autonomous loop tests: mock server returns tool calls, verify passes on Nth try
- T6 — Backend switching tests: verify correct URL/params per backend type

**Files**: `crates/anvil-llm/tests/`, `crates/anvil-agent/tests/`

---

## Implementation Order

The milestones are ordered by dependency and value. Each milestone
ships independently.

| Phase | Milestone | Stories | Key Deliverable |
|-------|-----------|---------|-----------------|
| 0 | Dev Environment | — | devcontainer.json, Rust toolchain |
| 1 | v0.1.1 Stability | S1-S3 | Ctrl+C, thinking mode, context compaction |
| 2 | v0.2.0 Interactive Autonomy | S4-S6 | Interactive Ralph Loop, backend lifecycle |
| 3 | v0.3.0 Context Intelligence | S7-S9 | Sliding window, session search, memory |
| 4 | v0.4.0 Multi-Model | S10-S12 | Model routing, parallel tools, cost tracking |
| 5 | v0.5.0 Resilience | S13-S15 | Error recovery, timeouts, validation |
| 6 | v0.6.0 Extensibility | S16-S18 | Custom tools, skill composition, hooks |
| 7 | v0.7.0 Performance | S19-S21 | Caching, streaming output, binary size |
| 8 | v0.8.0 Cross-Platform | S22-S24 | CI on Linux, macOS ARM, Windows/WSL |
| 9 | v0.9.0 Community | S25-S27 | Contributing guide, changelog, installer |
| 10 | v1.0.0 Daily Driver | S28-S30 | Soak test, config migration, offline help |

### Per-Milestone Definition of Done

Every milestone must satisfy:
1. All stories implemented with unit tests
2. Integration tests for key flows (where applicable)
3. All pre-existing tests still pass
4. `cargo clippy --all-targets -- -D warnings` clean
5. `cargo doc --no-deps` clean
6. CHANGELOG.md updated (from v0.9.0 onward)

---

## New Dependencies

| Crate | Purpose | Phase |
|-------|---------|-------|
| `wiremock` (dev) | Mock HTTP server for integration tests | 0 |
| `tokio-util` | CancellationToken (already in workspace) | 1 |
| `ctrlc` or raw signal handling | Ctrl+C detection | 1 |

No other new dependencies anticipated. Prefer existing crates over new ones.

---

## New Files Estimate (Cumulative)

| Category | Count |
|----------|-------|
| New Rust source files | ~12 |
| Modified Rust source files | ~20 |
| CI/CD workflows | 2-3 |
| Documentation files | 4-5 |
| Test fixtures | 5-10 |
| Scripts | 1-2 |

---

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Ctrl+C mid-stream leaves orphan HTTP connections | Resource leak | Drop reqwest response on cancel |
| Context compaction loses important info | User frustration | Preserve recent N messages, show before/after |
| Backend lifecycle: port conflicts | Startup failure | Check port availability before launch |
| Parallel tool execution: race conditions | Data corruption | Serialize mutating tools, parallelize read-only |
| Config migration: data loss | Broken setups | Backup before migration, validate after |
| Binary size: LTO increases build time | Slow CI | LTO only in release profile |
