# AGENTS.md — Anvil Master Prompt

This file is read by Anvil's system prompt builder (`system_prompt.rs`) and by
any AI agent bootstrapping into this codebase. It is the single source of truth
for project context.

---

## 1. Project Identity

**Anvil** is a terminal coding agent written in Rust. It connects to local LLM
backends (Ollama, llama-server, MLX) via the OpenAI-compatible chat completions
API. It runs offline, works airgapped, and never sends data to remote servers.

- **Repository**: https://github.com/baalho/anvil-tui
- **License**: Apache-2.0 (workspace `Cargo.toml`)
- **Rust edition**: 2021, MSRV 1.75
- **Current version**: 1.1.0
- **Target platform**: macOS (Apple Silicon primary), Linux, Windows/WSL
- **Default model**: `qwen3-coder:30b` (Ollama)

---

## 2. Architecture

### 2.1 Crate Dependency Graph

```
anvil-config ──┬──► anvil-llm ──┐
               │                ├──► anvil-agent ──► anvil (binary)
               └──► anvil-tools ┘
                                │
               anvil-mcp ───────┘
```

Dependencies flow left-to-right. `anvil-config` has no internal dependencies.
`anvil-mcp` depends on `anvil-config`. `anvil-agent` depends on all four
library crates. `anvil` is the CLI binary.

### 2.2 Crate Responsibilities

| Crate | Purpose | Key types |
|-------|---------|-----------|
| `anvil-config` | Settings, `.anvil/` harness, model profiles, bundled skills | `Settings`, `ProviderConfig`, `BackendKind`, `ModelProfile`, `SamplingConfig` |
| `anvil-llm` | OpenAI-compatible HTTP client, SSE streaming, retry | `LlmClient`, `ChatRequest`, `ChatResponse`, `StreamEvent`, `TokenUsage` |
| `anvil-tools` | 11 tools, executor, permissions, plugins, hooks, truncation | `ToolExecutor`, `PermissionHandler`, `TruncationConfig` |
| `anvil-mcp` | MCP client — JSON-RPC over stdio for external tool servers | `McpManager`, `McpServerConfig`, `McpTool` |
| `anvil-agent` | Agent loop, skills, personas, achievements, sessions, autonomous mode | `Agent`, `AgentEvent`, `Skill`, `SkillLoader`, `AutonomousRunner`, `Persona`, `AchievementStore` |
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
├── config.toml          # Provider, agent, tool, MCP settings
├── context.md           # Injected into system prompt (project info)
├── achievements.json    # Unlocked badges (persisted across sessions)
├── models/              # Per-model sampling profiles (TOML)
├── skills/              # Prompt template skills (Markdown + YAML frontmatter)
└── memory/              # Persistent learned patterns (categorized markdown)
```

Created by `anvil init`. Never committed to git (in `.gitignore`).

---

## 3. Current State (v1.1.0)

### What's built and working

- Interactive mode with readline-style input, streaming output, 14 slash commands
- 11 built-in tools: `shell`, `file_read`, `file_write`, `file_edit`, `grep`, `ls`, `find`, `git_status`, `git_diff`, `git_log`, `git_commit`
- MCP (Model Context Protocol) client for external tool servers via JSON-RPC over stdio
- Multi-backend support: Ollama, llama-server, MLX, Custom
- Model profiles with auto-applied sampling parameters
- Skills system with YAML frontmatter, env passthrough, verification commands, dependencies
- 17 bundled skills across infrastructure, dev-tools, meta, and kids categories
- Autonomous mode (Ralph Loop) with verification-based retry
- Session persistence in SQLite with resume (`anvil -c`), search, usage tracking
- Context compaction via LLM-generated summaries (`/clear`)
- Auto-compact when context usage exceeds configurable threshold
- Ctrl+C cancellation of in-flight LLM requests and tool execution
- Thinking mode parsing (`<think>` blocks from Qwen3/DeepSeek-R1) with `/think` toggle
- Backend lifecycle management (`/backend start llama <model>`, `/backend stop`)
- Model routing — route specific tools to different models (`/route`)
- Plugin system — user-defined tools via TOML in `.anvil/plugins/`
- Tool hooks — pre/post scripts for tool execution
- Character personas — themed system prompt wrappers for fun mode (`/persona`)
- Achievement system — 10 unlockable badges with persona-themed notifications
- Categorized project memory with search (`/memory search`, `/memory add category:convention`)
- Tool argument validation with actionable LLM error messages
- File cache with invalidation on write/edit
- Loop detection (hash-based, configurable limit)
- Output truncation (tail-truncation by lines/bytes, temp file fallback)
- Decoupled async TUI architecture (`--tui` flag) with input/engine/render tasks
- Universal `DynTool` trait for extensible tool system with STEM-ready structured output
- KV-cache-friendly layered system prompt (static → semi-static → dynamic)
- 246 tests, 0 clippy warnings, 0 doc warnings

### What's deferred

- Windows native support (WSL works)
- MCP server restart by name (requires config persistence refactor)

### Known issues

1. Ollama defaults to 2048 context tokens — set `OLLAMA_NUM_CTX` or use a
   model profile with `context.default_window`
2. MLX server tool calling varies by model — some models don't support it
3. GLM-4.7-Flash has chat template bugs on Ollama — use llama-server with `--jinja`

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
- Keep modules under 500 lines when possible (largest: `bundled_skills.rs`)
- Tests go in the same file for unit tests, `tests/` directory for integration tests

---

## 5. Development Workflow

### Build and test

```bash
cargo build                          # debug build (~3s)
cargo build --release                # release build (~9s, 13MB binary)
cargo test                           # all 246 tests (~7s)
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

## 8. Documentation

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

## 9. Key Files Reference

| File | What it does |
|------|-------------|
| `Cargo.toml` | Workspace root — all shared dependencies |
| `crates/anvil/src/main.rs` | CLI entry point, clap args, auto-detect, MCP init, Ralph Loop |
| `crates/anvil/src/commands.rs` | 14 slash command handlers (/help, /model, /backend, /skill, /mcp, /persona, /memory, etc.) |
| `crates/anvil/src/interactive.rs` | Readline loop, event processing, streaming display |
| `crates/anvil-agent/src/agent.rs` | Agent::turn() — the core loop with MCP tool dispatch |
| `crates/anvil-agent/src/skills.rs` | Skill parsing with YAML frontmatter and dependencies |
| `crates/anvil-agent/src/autonomous.rs` | Ralph Loop runner and verification |
| `crates/anvil-agent/src/persona.rs` | Character personas (Sparkle, Bolt, Codebeard) |
| `crates/anvil-agent/src/achievements.rs` | Achievement system with session tracking |
| `crates/anvil-agent/src/memory.rs` | Categorized project memory with search |
| `crates/anvil-agent/src/system_prompt.rs` | System prompt builder (reads this file) |
| `crates/anvil-config/src/profiles.rs` | Model profiles, sampling config, matching |
| `crates/anvil-config/src/provider.rs` | BackendKind enum, ProviderConfig |
| `crates/anvil-config/src/settings.rs` | Settings struct including MCP config |
| `crates/anvil-config/src/bundled_skills.rs` | 17 bundled skill file contents |
| `crates/anvil-llm/src/client.rs` | LlmClient — HTTP, streaming, retry, sampling injection |
| `crates/anvil-llm/src/stream.rs` | SSE parsing, ToolCallAccumulator |
| `crates/anvil-mcp/src/manager.rs` | MCP server manager — spawn, discover, dispatch, shutdown |
| `crates/anvil-tools/src/tools.rs` | 11 tool implementations (shell, file_read, git_status, etc.) |
| `crates/anvil-tools/src/tool_trait.rs` | Universal `DynTool` trait, `ToolRegistry`, STEM output types |
| `crates/anvil-tools/src/executor.rs` | Tool dispatch, validation, env passthrough |
| `crates/anvil-tools/src/plugins.rs` | User-defined tools via TOML plugin files |
| `crates/anvil-tools/src/hooks.rs` | Pre/post tool execution hooks |
| `crates/anvil/src/app.rs` | Decoupled async TUI (input/engine/render tasks) |
| `MANUAL.md` | User-facing usage guide |
| `CHANGELOG.md` | Version history |
