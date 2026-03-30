# AGENTS.md — Anvil Master Prompt

This file is read by Anvil's system prompt builder (`system_prompt.rs`) and by
any AI agent bootstrapping into this codebase. It is the single source of truth
for project context.

---

## 1. Project Identity

**Anvil** is a terminal coding agent written in Rust. It connects to local LLM
backends (Ollama, llama-server, MLX) via the OpenAI-compatible chat completions
API. It runs offline, works airgapped, and never sends data to remote servers.

- **Repository**: https://github.com/baalho/anvil-cli
- **License**: Apache-2.0 (workspace `Cargo.toml`)
- **Rust edition**: 2021, MSRV 1.75
- **Current version**: 0.1.0
- **Target platform**: macOS (Apple Silicon primary), Linux, Windows/WSL
- **Default model**: `qwen3-coder:30b` (Ollama)

---

## 2. Architecture

### 2.1 Crate Dependency Graph

```
anvil-config ──┬──► anvil-llm ──┐
               │                ├──► anvil-agent ──► anvil (binary)
               └──► anvil-tools ┘
```

Dependencies flow left-to-right. `anvil-config` has no internal dependencies.
`anvil-agent` depends on all three library crates. `anvil` is the CLI binary.

### 2.2 Crate Responsibilities

| Crate | Purpose | Key types |
|-------|---------|-----------|
| `anvil-config` | Settings, `.anvil/` harness, model profiles, bundled skills | `Settings`, `ProviderConfig`, `BackendKind`, `ModelProfile`, `SamplingConfig` |
| `anvil-llm` | OpenAI-compatible HTTP client, SSE streaming, retry | `LlmClient`, `ChatRequest`, `ChatResponse`, `StreamEvent`, `TokenUsage` |
| `anvil-tools` | 7 tools, executor, permissions, output truncation | `ToolExecutor`, `PermissionHandler`, `TruncationConfig` |
| `anvil-agent` | Agent loop, skills, system prompt, sessions, autonomous mode | `Agent`, `AgentEvent`, `Skill`, `SkillLoader`, `AutonomousRunner` |
| `anvil` | CLI binary, interactive mode, slash commands | `Cli` (clap), `Commands` |

### 2.3 Data Flow

```
User input
  → interactive.rs (readline loop)
  → Agent::turn() (agent.rs)
    → build ChatRequest with messages + tool definitions
    → LlmClient::chat_stream() (client.rs)
      → POST /v1/chat/completions (SSE)
      → parse StreamEvents (stream.rs)
    → if tool calls: ToolExecutor::execute() (executor.rs)
      → dispatch to tools.rs (file_read, shell, etc.)
      → truncate output (truncation.rs)
      → append tool result to messages
      → loop back to LLM
    → if no tool calls: emit TurnComplete
  → display to user
```

### 2.4 Key Abstractions

**BackendKind** (`provider.rs`): Enum with `Ollama`, `LlamaServer`, `Mlx`,
`Custom`. Serializes as kebab-case. Controls which model discovery endpoint
is used (`/api/tags` for Ollama, `/v1/models` for others).

**Model Profiles** (`profiles.rs`): TOML files in `.anvil/models/` with
`SamplingConfig` (temperature, top_p, min_p, repeat_penalty, top_k),
`ContextConfig` (max_window, default_window), and `BackendHints`. Matched
by case-insensitive substring against the active model name.

**Skills** (`skills.rs`): Markdown files with optional YAML frontmatter
(description, category, tags, env, verify). When activated, content is
injected into the system prompt and declared env vars are passed through
to the shell tool.

**Autonomous Mode** (`autonomous.rs`): The "Ralph Loop" — send prompt,
execute tools, run verification command, feed failure output back, repeat.
Guardrails: max iterations, max tokens, max wall-clock time. LLM can
declare `[ANVIL:DONE]` to trigger final verification.

**Session Persistence** (`session.rs`): SQLite via `rusqlite` (bundled).
Stores messages, tool calls, session metadata. Resume with `anvil -c`.

### 2.5 Harness Directory

```
.anvil/
├── config.toml     # Provider, agent, tool settings
├── context.md      # Injected into system prompt (lessons learned, project info)
├── models/         # Per-model sampling profiles (TOML)
├── skills/         # Prompt template skills (Markdown + YAML frontmatter)
└── memory/         # Reserved for future session memory
```

Created by `anvil init`. Never committed to git (in `.gitignore`).

---

## 3. Current State (v0.1.0)

### What's built and working

- Interactive mode with readline-style input, streaming output, slash commands
- 7 tools: `shell`, `file_read`, `file_write`, `file_edit`, `grep`, `ls`, `find`
- Multi-backend support: Ollama, llama-server, MLX, Custom
- Model profiles with auto-applied sampling parameters
- Skills system with YAML frontmatter, env passthrough, verification commands
- 14 bundled skills across infrastructure, dev-tools, and meta categories
- Autonomous mode (Ralph Loop) with verification-based retry
- Session persistence in SQLite with resume (`anvil -c`)
- Auto-detect model on startup (queries backend for available models)
- Retry with exponential backoff (Retryable vs Permanent error distinction)
- Context window estimation with 80% warning
- Loop detection (hash-based, configurable limit)
- Output truncation (tail-truncation by lines/bytes, temp file fallback)
- 91 tests, 0 clippy warnings, 0 doc warnings

### What's deferred (see AGILE.md)

- Context compaction (`/clear` is a placeholder)
- Ctrl+C cancellation of in-flight LLM requests
- Thinking mode parsing (`<think>` blocks from Qwen3/DeepSeek-R1)
- Interactive Ralph Loop (`/ralph` command shows help only)
- Backend lifecycle management (start/stop llama-server from Anvil)
- Plugin/extension system
- Windows native support (WSL works)

### Known issues

1. Ollama defaults to 2048 context tokens — set `OLLAMA_NUM_CTX` or use a
   model profile with `context.default_window`
2. MLX server tool calling varies by model — some models don't support it
3. Qwen3/DeepSeek-R1 `<think>` blocks appear in raw output (not parsed/hidden)
4. GLM-4.7-Flash has chat template bugs on Ollama — use llama-server with `--jinja`

---

## 4. Code Conventions

### 4.1 Rust Style

- **Edition 2021**, resolver 2
- **Error handling**: `anyhow::Result` everywhere. Use `bail!()` for early returns.
  Use `?` propagation. No `unwrap()` in production code (only in tests).
- **Async**: `tokio` runtime with `features = ["full"]`. Async functions for I/O,
  sync for pure computation.
- **Serialization**: `serde` with derive. JSON for API wire format, TOML for config
  files, YAML for skill frontmatter.
- **Naming**: snake_case for functions/variables, CamelCase for types, SCREAMING_SNAKE
  for constants. Module names match the primary type they export.
- **Imports**: workspace dependencies in root `Cargo.toml`, crate-level re-exports
  in `lib.rs`. Internal crates referenced by name (`anvil_config::BackendKind`).
- **Doc comments**: `///` on all public items. `//!` for module-level docs.
  Document the "why", not the "what". Include `# Why` sections for non-obvious
  design decisions.

### 4.2 Testing Patterns

- Unit tests in `#[cfg(test)] mod tests` at the bottom of each file
- Integration tests in `crates/*/tests/` directories
- Use `tempfile::TempDir` for filesystem tests — never write to real paths
- Test names describe the behavior: `match_profile_case_insensitive`,
  `skip_invalid_profile_files`, `backward_compatible_no_frontmatter`
- Assert specific values, not just `is_some()` / `is_ok()`

### 4.3 Commit Messages

Follow the pattern established in the repo:
```
<scope>: <what changed>
```
Examples from history:
- `M0: working agent MVP`
- `Rewrite ONBOARDING.md for Rust beginners`
- `Implement exercises 1.1 and 1.2, upgrade 1.2 to guided walkthrough`

### 4.4 File Organization

- One primary type per module (e.g., `client.rs` → `LlmClient`)
- Re-export public API from `lib.rs`
- Keep modules under 500 lines when possible (largest: `bundled_skills.rs` at 690)
- Tests go in the same file for unit tests, `tests/` directory for integration tests

---

## 5. Development Workflow

### Build and test

```bash
cargo build                          # debug build (~3s)
cargo build --release                # release build (~9s, 13MB binary)
cargo test                           # all 91 tests (~2s)
cargo clippy --all-targets -- -D warnings
cargo doc --no-deps                  # generate docs (zero warnings)
```

### Add a new tool

1. Add the tool function in `crates/anvil-tools/src/tools.rs`
2. Add its JSON schema in `crates/anvil-tools/src/definitions.rs`
3. Add dispatch in `crates/anvil-tools/src/executor.rs`
4. Classify as read-only or mutating in `crates/anvil-tools/src/permission.rs`
5. Add tests in `crates/anvil-tools/tests/tool_tests.rs`

### Add a new slash command

1. Add the command handler in `crates/anvil/src/commands.rs`
2. Add it to the `match` in `handle_command()`
3. Add it to `help_text()`

### Add a new model profile

1. Create `crates/anvil-config/src/profiles.rs` entry in `BUNDLED_PROFILES`
2. Add the TOML file content with `name`, `match_patterns`, `[sampling]`,
   `[context]`, and `[backend]` sections
3. Run `cargo test` — the `parse_bundled_profiles` test validates all entries

### Add a new skill

1. Create a markdown file in the `BUNDLED_SKILLS` constant
   (`crates/anvil-config/src/bundled_skills.rs`)
2. Optional: add YAML frontmatter with `description`, `category`, `tags`,
   `env`, `verify`
3. The skill is installed by `anvil init` into `.anvil/skills/`

---

## 6. Lessons Learned

These are hard-won patterns from development. Violating them will cause bugs.

### Shell commands must be strings, not argv arrays
LLMs generate `ls -la /tmp`, not `["ls", "-la", "/tmp"]`. The shell tool
accepts a string and runs it via `sh -c`. This was the single highest-impact
fix — 100% of shell tool failures were caused by the original argv format.

### Readline over TUI
A ratatui TUI was built and deleted. It blocked during LLM streaming, had
race conditions, and was 3x the code. The current readline-style interface
(crossterm for colors, raw mode for single-keypress prompts) is simpler
and more reliable.

### Option\<Agent\> pattern for async ownership
Rust's ownership rules prevent moving `&mut self` into a spawned task.
The `Option<Agent>` take/put pattern solves this:
```rust
let agent = self.agent.take().unwrap();
let handle = tokio::spawn(async move { agent.turn(...).await });
self.agent = Some(handle.await??);
```

### Tail-truncation, not head-truncation
When tool output exceeds limits, the tail (most recent output) is more
useful than the head. Full output is saved to a temp file as fallback.

### Retry: Retryable vs Permanent
Not all errors should be retried. 404 (model not found) is permanent.
429 (rate limit) is retryable. The `RetryError` enum makes this explicit.

### Model profiles as TOML files
Different models need different sampling params. TOML files in `.anvil/models/`
are user-editable without recompilation.

### env_clear() with escape hatch
Shell commands use `env_clear()` for security, passing only safe vars
(PATH, HOME, etc.). Skills declare additional env vars in frontmatter
for passthrough. This balances security with functionality.

### Don't assume model capabilities
Small models (7B) may ignore complex instructions. Keep tool definitions
simple. Test with the smallest model you plan to support.

### Ollama context window gotcha
Ollama defaults to 2048 context tokens regardless of the model's capability.
Set `OLLAMA_NUM_CTX=32768` or use model profiles with `context.default_window`.

### GLM-4.7-Flash needs llama-server
Unsloth warns against Ollama for GLM-4.7-Flash due to chat template bugs.
Use llama-server with `--jinja`. Set `repeat_penalty = 1.0` (must be disabled).

---

## 7. Devil's Advocate Checklist

Before making any change, ask yourself:

1. **Does this break existing tests?** Run `cargo test` before and after.
2. **Does this work on macOS AND Linux?** Anvil targets both. Use `#[cfg(unix)]`
   for platform-specific code, not `#[cfg(target_os = "macos")]`.
3. **Is this the simplest solution?** The TUI was deleted because readline was
   simpler. Prefer boring solutions.
4. **Does this respect the crate boundary?** `anvil-config` has no internal
   dependencies. Don't add one.
5. **Will this work with all backends?** Ollama, llama-server, and MLX have
   different quirks. Test the change against at least two.
6. **Does the LLM actually generate this format?** Test with a real model.
   LLMs don't read your code — they generate what they've seen in training data.
7. **Am I adding a dependency?** Check if it's already in workspace `Cargo.toml`.
   Prefer existing deps over new ones. Anvil should stay lean.
8. **Does this handle the error case?** Use `anyhow::Result` and `bail!()`.
   Never `unwrap()` in production code.
9. **Is the doc comment explaining "why", not "what"?** `/// Increment counter`
   is noise. `/// Increment to track retry attempts for backoff calculation` is signal.
10. **Will this confuse a user who runs `anvil init` for the first time?**
    Defaults should work out of the box with just Ollama installed.

---

## 8. Project Plan

See [AGILE.md](AGILE.md) for the feature-driven roadmap from v0.1.1 to v1.0.

---

## 9. Documentation

API documentation is generated from code comments via `cargo doc`:

```bash
cargo doc --no-deps --open           # local preview
cargo doc --no-deps --document-private-items  # include internals
```

GitHub Pages deployment is configured in `.github/workflows/docs.yml` —
triggers on push to `master`.

Every public item has `///` doc comments. Module-level docs use `//!`.
The codebase produces zero doc warnings with `cargo doc --no-deps`.

---

## 10. Key Files Reference

| File | What it does |
|------|-------------|
| `Cargo.toml` | Workspace root — all shared dependencies |
| `crates/anvil/src/main.rs` | CLI entry point, clap args, auto-detect, Ralph Loop |
| `crates/anvil/src/commands.rs` | Slash command handlers (/help, /model, /backend, /skill) |
| `crates/anvil/src/interactive.rs` | Readline loop, event processing, streaming display |
| `crates/anvil-agent/src/agent.rs` | Agent::turn() — the core loop |
| `crates/anvil-agent/src/skills.rs` | Skill parsing with YAML frontmatter |
| `crates/anvil-agent/src/autonomous.rs` | Ralph Loop runner and verification |
| `crates/anvil-agent/src/system_prompt.rs` | System prompt builder (reads this file) |
| `crates/anvil-config/src/profiles.rs` | Model profiles, sampling config, matching |
| `crates/anvil-config/src/provider.rs` | BackendKind enum, ProviderConfig |
| `crates/anvil-config/src/bundled_skills.rs` | 14 bundled skill file contents |
| `crates/anvil-llm/src/client.rs` | LlmClient — HTTP, streaming, retry, sampling injection |
| `crates/anvil-llm/src/stream.rs` | SSE parsing, ToolCallAccumulator |
| `crates/anvil-tools/src/tools.rs` | Tool implementations (shell, file_read, etc.) |
| `crates/anvil-tools/src/executor.rs` | Tool dispatch, env passthrough |
| `LESSONS_LEARNED.md` | What worked, what didn't, patterns to reuse |
| `MANUAL.md` | User-facing usage guide |
| `AGILE.md` | Feature roadmap v0.1.1 → v1.0 |
