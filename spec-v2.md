# Anvil — Feature-Complete Spec

## Vision

A polished, daily-driver coding agent for local models. Depth over breadth.
Anvil connects to Ollama, runs entirely offline, and does a small number of
things extremely well: read code, write code, run commands, search, and
follow user-defined skills.

Design reference: pi.dev's coding-agent — powerful yet simple.

---

## Problem Statement

The current M0 implementation works but has gaps that prevent daily use:

1. **TUI is broken for real work** — ratatui blocks during generation, no
   streaming visible to user, event loop races with permission prompts.
2. **Single-shot sessions** — no way to resume a conversation.
3. **No retry/error recovery** — one HTTP failure kills the session.
4. **Crude output handling** — character-truncation loses context; no temp
   file fallback.
5. **No skills** — the `.anvil/skills/` directory exists but nothing loads it.
6. **Missing tools** — no `ls`, no `find`, no way to explore a project
   without shell.
7. **Shell tool is argv-only** — models consistently generate shell strings,
   not argv arrays. Every Ollama model fails at this.
8. **No context window management** — long sessions silently exceed the
   model's context and produce garbage.

---

## Scope

### In Scope

| Area | What |
|---|---|
| **Interface** | Replace ratatui TUI with readline-style interactive mode (stdin/stdout, colors, streaming). Keep non-interactive `run` mode. |
| **Tools** | Fix shell (accept string commands), add `ls` and `find` tools, improve output truncation with temp file fallback. |
| **Skills** | Load markdown files from `.anvil/skills/`, inject into system prompt on demand via `/skill` command. |
| **Sessions** | Resume last session (`anvil --continue`), list and pick sessions. |
| **Reliability** | Retry with exponential backoff, stream error recovery, graceful timeout handling. |
| **Context management** | Track token usage against model context window, auto-compact when approaching limit. |
| **System prompt** | Richer prompt with date, OS, working directory. Load `.goosehints`, `AGENTS.md`, `CLAUDE.md` for compatibility. |
| **Slash commands** | `/help`, `/history`, `/skill`, `/model`, `/stats`, `/clear`, `/end`. |
| **Loop detection** | Detect repeated identical tool calls, pause and ask user. |
| **Better error messages** | Malformed tool call recovery, clear error display. |

### Out of Scope

- Multi-provider (Anthropic, OpenAI, Google native APIs) — Ollama only
- MCP client/server
- Container sandboxing (Podman/Docker)
- Heartbeat/background scheduler
- Embeddings/semantic memory
- Git snapshots and undo
- Named checkpoints and session forking
- Web fetch tool
- Prompt injection defense (Unicode sanitization, boundary markers)
- Secret scanning
- Learning curriculum / ONBOARDING.md updates

---

## Architecture

### Crate Structure (unchanged)

```
crates/
├── anvil-config    # Settings, harness, provider config
├── anvil-llm       # LLM client, streaming, retry, usage tracking
├── anvil-tools     # Tool implementations, permissions, executor
├── anvil-agent     # Agent loop, session store, system prompt, skills
└── anvil           # CLI binary, interactive mode, slash commands
```

No new crates. The existing 5-crate structure is sufficient.

---

## Detailed Requirements

### 1. Interactive Mode (replace ratatui TUI)

**Current state:** `tui.rs` uses ratatui with raw mode, alternate screen,
custom event loop. It blocks during generation, doesn't stream output
properly, and has race conditions with permission prompts.

**Target state:** Simple readline-style interface inspired by pi/Claude Code.

**Behavior:**
- Print a welcome banner with version, session ID, model name, working dir.
- Prompt: `you> ` for user input, `anvil> ` prefix for assistant output.
- Stream assistant text token-by-token to stdout as it arrives.
- Tool calls: print `[tool: name(args)]` in dim/yellow, then result summary.
- Permission prompts: inline `Allow shell: "cargo build"? [y/n/a] ` — block
  on single keypress via `crossterm` raw mode (enable raw mode only for the
  single keypress, then disable immediately).
- Slash commands start with `/` and are handled before sending to LLM.
- Ctrl+C during generation: cancel current request via `CancellationToken`,
  keep session alive.
- Ctrl+C at prompt: exit gracefully (set session to Paused).
- Multi-line input: backslash continuation (`\` at end of line).

**Implementation approach:**
- Use `std::io::stdin().read_line()` for normal input (no new dependency).
  Input history is not in scope for v1 — keep it simple.
- For single-keypress permission prompts: temporarily enable `crossterm`
  raw mode, read one key, disable raw mode. `crossterm` is already a dep.
- Async: spawn agent turn on tokio, receive events via `mpsc::channel`,
  print to stdout from the event-receiving task.
- Colors via `crossterm::style` (already a dependency).
- Cancellation via `tokio_util::sync::CancellationToken` (already a dep).

**Acceptance criteria:**
1. Streaming text appears token-by-token during generation.
2. Permission prompt responds to single keypress (y/n/a).
3. Ctrl+C cancels generation without killing the process.
4. Works on Windows (cmd.exe, PowerShell) and Unix terminals.

### 2. Shell Tool Fix

**Current state:** Shell tool requires `command` as an argv array
(`["ls", "-la"]`). Every Ollama model generates shell strings instead
(`"ls -la"`), causing 100% tool call failure.

**Target state:** Accept string commands, execute via system shell.
Drop the argv array format entirely — it's dead code that no model uses.

**Behavior:**
- `command` parameter: **string only**.
- Execute via `sh -c "command"` on Unix, `cmd.exe /C "command"` on Windows.
- Timeout: per-call `timeout` parameter (optional integer, seconds), falls
  back to config default (`shell_timeout_secs`).
- Output handling: delegate to the shared truncation system (Section 11).
- Process cleanup: on timeout or cancellation, kill the child process.
  On Unix, kill the process group (`libc::killpg` or `nix`). On Windows,
  `taskkill /T`. If neither is available, fall back to `child.kill()`.
- Environment: keep the existing `env_clear()` + safe vars approach.

**Tool definition:**
```json
{
  "name": "shell",
  "description": "Execute a shell command. Returns stdout, stderr, and exit code.",
  "parameters": {
    "type": "object",
    "properties": {
      "command": {
        "type": "string",
        "description": "Shell command to execute"
      },
      "timeout": {
        "type": "integer",
        "description": "Timeout in seconds (optional)"
      }
    },
    "required": ["command"]
  }
}
```

**Acceptance criteria:**
1. `{"command": "ls -la src/"}` works.
2. `{"command": "echo hello && echo world"}` works (shell interpretation).
3. Timed-out commands report timeout to the LLM with partial output.
4. Works on both Unix and Windows.

### 3. New Tools: `ls` and `find`

**`ls` tool:**
- List directory contents with file type indicator and sizes.
- Respects workspace boundary (reuses `resolve_path`).
- Skips `.git`, `node_modules`, `target`, `__pycache__` by default.
- Optional `all` boolean parameter to include hidden files.
- Returns one entry per line: `dir   src/`, `file  main.rs  (1.2 KB)`.

**`find` tool:**
- Find files matching a glob pattern.
- Recursive with configurable `max_depth` (default 10).
- Respects workspace boundary and same skip patterns as `ls`.
- Returns list of matching relative paths, one per line.
- Limit results to 500 entries (with truncation notice).

**Acceptance criteria:**
1. `ls` on project root shows files and directories with sizes.
2. `find` with `"*.rs"` pattern returns all Rust files.
3. Both tools refuse paths outside workspace.
4. Both tools skip `.git`, `node_modules`, `target` by default.

### 4. Skills System

**Current state:** `.anvil/skills/` directory is created by `init` but
nothing reads it.

**Target state:** Markdown files in `.anvil/skills/` are loadable prompt
templates that inject context into the system prompt.

**Behavior:**
- Skill files: `.anvil/skills/<name>.md` — plain markdown.
- Discovery: scan `.anvil/skills/` at startup, list available skills.
- Activation: `/skill <name>` command injects the skill's content into the
  system prompt for the current session.
- Deactivation: `/skill clear` removes all injected skill content.
- Multiple skills: can stack (append in order of activation).
- Listing: `/skill` with no args lists available skills. Each skill shows
  its filename and the first non-empty, non-heading line as description.

**Skill file format:**
```markdown
# PR Review

Review the pull request changes. Focus on:
- Logic errors and edge cases
- Security issues
- Performance concerns

Output a structured review with severity ratings.
```

The first `# heading` is the display name. The filename (minus `.md`) is
the activation key. Everything after the heading is the prompt content.

**Where skills live in the code:**
- New module `crates/anvil-agent/src/skills.rs` — `SkillLoader` struct
  with `scan()`, `get()`, `list()` methods.
- `build_system_prompt()` accepts `active_skills: &[Skill]` parameter.
- `Agent` struct holds `active_skills: Vec<Skill>`.

**Acceptance criteria:**
1. `/skill` lists available skills from `.anvil/skills/`.
2. `/skill pr-review` injects the skill content into the system prompt.
3. `/skill clear` removes all injected skills.
4. Skills persist across turns within a session.
5. Missing skill name prints "skill not found: <name>".

### 5. Session Resume

**Current state:** Sessions are saved to SQLite but cannot be resumed.

**Target state:** Resume the last session or a specific session by ID.

**Behavior:**
- `anvil --continue` or `anvil -c`: resume the most recent non-completed
  session. Reload messages from SQLite, reconstruct `Vec<ChatMessage>`,
  display a summary line.
- `anvil --continue <id-prefix>`: resume by ID prefix match (8+ chars).
- On resume: regenerate the system prompt (picks up updated context.md,
  new skills, etc.). Prepend it as the first message. Then append all
  stored user/assistant/tool messages in order.
- Display on resume: "Resuming session <id> (<N> messages, started <time>)"
  followed by the last 3 messages as context.
- Session status transitions: Active → Paused (on normal exit) → Active
  (on resume). Completed sessions cannot be resumed.
- Stale warning: if session is older than 24h, print warning but proceed.

**Message reconstruction from SQLite:**
`StoredMessage` → `ChatMessage` mapping:
- `role="user"` → `ChatMessage::user(content)`
- `role="assistant"` with `tool_calls_json` → `ChatMessage` with
  deserialized `tool_calls` field
- `role="tool"` → `ChatMessage::tool_result(tool_call_id, content)`
- `role="system"` → skip (regenerated fresh)

**New `SessionStore` methods needed:**
- `find_latest_resumable() -> Option<Session>` — most recent Active/Paused.
- `find_by_prefix(prefix: &str) -> Option<Session>` — ID prefix match.

**Acceptance criteria:**
1. `anvil -c` resumes the last session with full conversation context.
2. `anvil -c abc12345` resumes a specific session.
3. Resumed session can continue with new prompts and tool calls.
4. Exiting sets status to Paused; `/end` sets status to Completed.
5. Completed sessions are not resumable.

### 6. Retry and Error Recovery

**Current state:** One HTTP error kills the session.

**Target state:** Automatic retry with exponential backoff for transient
errors.

**Behavior:**
- Retry on: HTTP 429, 500, 502, 503, 504, network timeout, connection
  reset.
- Do not retry: HTTP 400, 401, 403, 404, or any response with a parseable
  error body indicating a permanent failure.
- Config: max 3 retries, initial delay 1s, backoff multiplier 2x, max
  delay 30s.
- Jitter: ±20% using simple timestamp-based pseudo-randomness (no `rand`
  crate needed — `SystemTime::now().as_nanos() % 40` gives 0-39, map to
  0.8-1.2 multiplier).
- On retry: emit `AgentEvent::Retry { attempt, max, delay_secs }` so the
  UI can print `[retrying in 2s... (attempt 2/3)]`.
- After all retries exhausted: emit `AgentEvent::Error` with the last
  error message. Session stays alive — user can try again.
- Stream interruption: if SSE stream breaks mid-response, the accumulated
  `content_buf` and `tool_acc` state is preserved. Emit what we have as a
  partial assistant message, then emit an error event.

**Implementation:**
- New `crates/anvil-llm/src/retry.rs` with `RetryConfig` struct and
  `retry_async()` function.
- `LlmClient::chat_stream()` wraps the HTTP POST in `retry_async()`.
  Only the initial HTTP request is retried, not the stream processing.

**Acceptance criteria:**
1. HTTP 429 triggers automatic retry with increasing delay.
2. HTTP 401 fails immediately with "authentication error" message.
3. After 3 failed retries, session stays alive for user to act.
4. Retry attempts are visible to the user.

### 7. Context Window Management

**Current state:** No awareness of model context limits.

**Target state:** Track estimated token usage, warn user, auto-compact.

**Behavior:**
- Token estimation: `content.len() / 4` as rough approximation. Good
  enough for warnings — not used for billing.
- Context limit: query Ollama `/api/show` endpoint for the model's
  `num_ctx` parameter at startup. Fall back to config value
  `context_window` (default 8192) if query fails.
- Warning: when estimated context exceeds 80% of limit, print a one-line
  warning after the assistant's response: `[context: ~6500/8192 tokens]`.
- Auto-compaction at 90%: send a compaction prompt to the LLM asking it
  to summarize the conversation. Replace all messages (except system
  prompt) with a single user message containing the summary, plus a
  system note "[conversation compacted]".
- If compaction itself fails (LLM error), warn the user and continue
  without compacting. Do not retry compaction.
- Manual compaction: `/clear` triggers compaction immediately.
- `/stats` shows: estimated context usage, token counts, session duration.

**Acceptance criteria:**
1. Context warning appears when exceeding 80%.
2. Auto-compaction triggers at 90% and reduces message count.
3. Compacted sessions continue working.
4. `/stats` shows context usage.
5. Compaction failure doesn't crash the session.

### 8. Slash Commands

Available in interactive mode only:

| Command | Action |
|---|---|
| `/help` | List available commands |
| `/history` | List recent sessions (reuses existing logic) |
| `/skill [name]` | List or activate a skill |
| `/skill clear` | Deactivate all skills |
| `/model [name]` | Show current model or switch to another Ollama model |
| `/stats` | Token usage, estimated context %, session duration |
| `/clear` | Compact conversation context |
| `/end` | Mark session completed and exit |

**`/model` details:**
- `/model` with no args: print current model name.
- `/model <name>`: switch to a different Ollama model. Query
  `/api/tags` to verify the model exists. Update `LlmClient` config.
  Re-query context window size. Print confirmation.
- Switching models does NOT clear the conversation — the existing
  messages stay. The new model continues from the same context.

**Implementation:**
- New `crates/anvil/src/commands.rs` module with a `handle_command()`
  function that pattern-matches on the command string.
- Returns `CommandResult` enum: `Handled`, `Exit`, `Unknown(String)`.

**Acceptance criteria:**
1. All commands listed above work.
2. Unknown `/xyz` prints "unknown command, try /help".
3. Commands are not sent to the LLM.
4. `/model` without Ollama running prints a connection error.

### 9. System Prompt Improvements

**Current state:** Basic prompt with tool descriptions and project context.

**Target state:** Richer prompt with environment info and compatibility
file loading.

**Additions to `build_system_prompt()`:**
- Current date: `YYYY-MM-DD`.
- Operating system: `std::env::consts::OS` and `std::env::consts::ARCH`.
- Working directory: absolute path.
- Model name: from config.
- Compatibility files: load content from these files if they exist in the
  workspace root (checked in order, all included if present):
  `.goosehints`, `AGENTS.md`, `CLAUDE.md`, `.cursorrules`,
  `.github/copilot-instructions.md`.
- Active skills: appended as `## Skill: <name>` sections.
- Better tool guidelines (adapted from pi):
  "Read files before editing. Use edit for precise changes, write only for
  new files or complete rewrites. Show file paths when working with files."

**Function signature change:**
```rust
pub fn build_system_prompt(
    workspace: &Path,
    override_prompt: Option<&str>,
    model_name: &str,
    active_skills: &[Skill],
) -> String
```

**Acceptance criteria:**
1. System prompt includes date, OS, working directory, model name.
2. `.goosehints` content appears in system prompt when file exists.
3. Active skills appear as labeled sections.
4. Missing compatibility files are silently skipped.

### 10. Loop Detection

**Current state:** `loop_detection_limit` exists in config but is not
enforced.

**Target state:** Detect and break infinite tool call loops.

**Behavior:**
- `Agent` maintains a `Vec<u64>` of recent tool call hashes
  (hash of `tool_name + arguments`).
- After each tool call, push the hash. Keep last 20 entries.
- If the same hash appears `loop_detection_limit` times (default 5) in
  the window, emit `AgentEvent::LoopDetected { tool_name, count }`.
- The interactive mode handles this by prompting: "Detected repeated
  call: shell('cargo build') x5. Continue? [y/n]".
- On "n": inject a tool result message saying "Loop detected by user.
  Try a different approach." and continue the agent loop.
- On "y": reset the counter for that hash and continue.

**Acceptance criteria:**
1. 5 identical tool calls triggers the loop warning.
2. User can continue or break the loop.
3. Breaking the loop informs the LLM to try differently.

### 11. Output Truncation (shared system)

**Current state:** Simple character-count truncation in `ToolExecutor`.

**Target state:** Tail-truncation by lines and bytes, with temp file
fallback. Used by all tools.

**Behavior:**
- Configurable limits: `max_output_lines` (default 200),
  `max_output_bytes` (default 30,000).
- Truncation keeps the **tail** (last N lines) — end of output is most
  relevant for build errors, test results, etc.
- If truncated: write full output to a temp file
  (`std::env::temp_dir().join(format!("anvil-{}.log", uuid))`), append
  notice to the truncated output:
  `[Showing lines X-Y of Z. Full output: /tmp/anvil-xxxx.log]`.
- Applies to all tool output via `ToolExecutor::truncate_output()`.

**Replaces** the current `truncate_output()` method which does simple
character slicing.

**Acceptance criteria:**
1. Output over 200 lines shows only the last 200 lines.
2. Full output is saved to a readable temp file.
3. The truncation notice includes the temp file path.
4. Works for shell, grep, ls, find output.

### 12. Non-Interactive Mode Improvements

**Current state:** `anvil run --prompt "..."` auto-approves all tools.

**Target state:** Explicit `--yes` flag for auto-approve.

**Behavior:**
- `anvil run --prompt "..." --yes` or `-y`: auto-approve all tools.
- `anvil run --prompt "..."` (no flag): prompt for permission on
  stdin/stdout using the same single-keypress mechanism.
- `anvil run --prompt "..." --output json`: wrap final assistant response
  in JSON: `{"content": "...", "tool_calls": [...], "usage": {...}}`.
- Exit code: 0 on success, 1 on error.

**Acceptance criteria:**
1. Without `--yes`, tool calls prompt for permission.
2. `--yes` auto-approves silently.
3. `--output json` produces valid JSON.

### 13. Agent Loop: Cancellation Support

**Current state:** `Agent::turn()` runs to completion. No way to cancel.

**Target state:** Cancellation token threaded through the agent loop.

**Behavior:**
- `Agent::turn()` accepts a `CancellationToken` parameter.
- Before each LLM call: check if cancelled. If so, return early.
- During streaming: `tokio::select!` between the stream and the token.
  On cancellation, drop the stream, emit partial content if any.
- During tool execution: pass cancellation to `tokio::time::timeout`.
  On cancellation, kill the child process.
- Emit `AgentEvent::Cancelled` when cancellation is handled.

**Acceptance criteria:**
1. Ctrl+C during generation stops the LLM stream within 1 second.
2. Ctrl+C during tool execution kills the running process.
3. Session remains usable after cancellation.

---

## Implementation Plan

Ordered by dependency. Each phase produces a working, testable binary.

### Phase 1: Foundation Fixes

1. **Fix shell tool** — string commands via system shell. Drop argv array.
2. **Shared output truncation** — tail-truncation with temp file fallback.
3. **Add `ls` and `find` tools** — with workspace boundary enforcement.
4. **Retry logic** — `RetryConfig` + `retry_async()` in `anvil-llm`.
5. **Unit tests** for shell, truncation, ls, find, retry.

### Phase 2: Interactive Mode Rewrite

6. **Replace ratatui TUI** — new `interactive.rs`. Streaming output,
   colored prefixes, single-keypress permission prompts via crossterm.
7. **Slash commands** — `/help`, `/stats`, `/end`, `/clear`, `/history`,
   `/model`, `/skill`.
8. **Cancellation** — `CancellationToken` through agent loop, Ctrl+C
   handler in interactive mode.
9. **Non-interactive improvements** — `--yes` flag, JSON output.

### Phase 3: Session Resume + Context Management

10. **Session resume** — `--continue` flag, message reconstruction,
    status transitions.
11. **Context window tracking** — token estimation, Ollama `/api/show`
    query, warning at 80%, auto-compaction at 90%.
12. **Loop detection** — hash-based tracking, user prompt on detection.

### Phase 4: Skills + System Prompt

13. **Skills loader** — `skills.rs` module, scan/parse/list/get.
14. **System prompt enrichment** — date, OS, cwd, model, compatibility
    files, skill injection.
15. **`/skill` command** — activate, deactivate, list.

### Phase 5: Polish + Testing

16. **Integration tests** — mock HTTP server returning canned SSE,
    test full agent loop with tool calls and streaming.
17. **Additional unit tests** — skills, session resume, context
    estimation, loop detection, slash commands.
18. **Error handling** — malformed tool call recovery (try to parse
    arguments even if JSON is slightly broken), clear error display.
19. **README** — practical usage guide with examples.

---

## Acceptance Criteria (Overall)

1. `anvil` starts interactive mode with streaming output visible
   token-by-token.
2. Shell commands work with string input (`"ls -la"` not `["ls", "-la"]`).
3. `/skill pr-review` loads a skill and affects agent behavior.
4. `anvil -c` resumes the last session with full context.
5. HTTP 429 from Ollama triggers automatic retry with backoff.
6. Long sessions auto-compact without user intervention.
7. Repeated identical tool calls trigger a warning.
8. Ctrl+C cancels generation without killing the session.
9. `cargo clippy --all-targets -- -D warnings` passes.
10. `cargo test` passes with >30 tests across all crates.
11. Works on Windows (PowerShell) and Unix terminals.

---

## Dependencies

**Removed:** `ratatui` (replaced by direct stdout + crossterm).

**No new crates added.** The existing set covers all requirements:
- `crossterm` — colors, single-keypress input (already a dep)
- `tokio-util` — `CancellationToken` (already a dep)
- `reqwest` — HTTP for Ollama API queries (already a dep)
- `uuid` — temp file naming (already a dep)

Jitter for retry uses timestamp-based pseudo-randomness, avoiding a
`rand` dependency.

---

## What This Is Not

- Not a multi-provider agent (Ollama only, OpenAI-compatible API)
- Not an MCP client/server
- Not a sandboxed execution environment
- Not a learning curriculum
- Not a web UI or Electron app

Anvil is a terminal coding agent that does one thing well: help you write
code using local models.
