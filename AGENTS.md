# AGENTS.md — Anvil

Single source of truth for AI agents working in this codebase.

---

## 1. Identity

**Anvil** — terminal coding agent in Rust. Connects to local LLM backends
(Ollama, llama-server, MLX) via OpenAI-compatible API. Offline-first, airgapped.

- **Repo**: https://github.com/baalho/anvil-tui
- **License**: Apache-2.0
- **Rust**: edition 2021, MSRV 1.75
- **Version**: 2.2.0
- **Platforms**: macOS, Linux, Windows/WSL
- **Default model**: `qwen3-coder:30b` (Ollama)

---

## 2. Architecture

### Crate graph

```
anvil-config ──┬──► anvil-llm ──┐
               │                ├──► anvil-agent ──► anvil (binary)
               └──► anvil-tools ┘         │
                                          │
               anvil-mcp ────────────────┘
```

`anvil-config` has no internal deps. `anvil-mcp` has no internal deps.
`anvil-tools` depends on `anvil-config`. `anvil` binary depends on
`anvil-config`, `anvil-llm`, `anvil-tools`, and `anvil-agent`.

### Crates

| Crate | Purpose |
|-------|---------|
| `anvil-config` | Settings, `.anvil/` harness, model profiles, bundled skills/layouts, inventory |
| `anvil-llm` | HTTP client, SSE streaming, retry, sampling injection, tool_choice, MLX fallback |
| `anvil-tools` | 11 tools, executor, permissions, plugins, hooks, truncation, ToolOutput |
| `anvil-mcp` | MCP client — JSON-RPC over stdio |
| `anvil-agent` | Agent loop, skills, personas, modes, achievements, sessions, autonomous mode |
| `anvil` | CLI binary, interactive mode, 17 slash commands, Renderer trait, launch profiles |

### Key abstractions

**BackendKind** (`provider.rs`): `Ollama`, `LlamaServer`, `Mlx`, `Custom`.
Controls model discovery endpoint (`/api/tags` vs `/v1/models`).

**Model Profiles** (`profiles.rs`): TOML in `.anvil/models/` with sampling,
context, backend hints, and capability tags. Matched by case-insensitive substring.

**Mode** (`mode.rs`): `Coding` or `Creative`. Controls `tool_choice` in API
requests and whether tools are sent. Personas auto-set mode (kids → Creative,
homelab → Coding). User overrides with `/mode`.

**ToolChoice** (`message.rs`): `auto`, `none`, `required`, or specific function.
Sent in every `ChatRequest` to tell the model whether to use tools.

**Skills** (`skills.rs`): Markdown + YAML frontmatter. Injected into system
prompt when activated. Env vars declared in frontmatter pass through to shell.

**Inventory** (`inventory.rs`): TOML in `.anvil/inventory.toml`. Host/service
registry injected into system prompt for infrastructure management.

**Autonomous Mode** (`autonomous.rs`): Send prompt → execute tools → run
verify command → feed failure back → repeat. Guardrails: max iterations,
tokens, wall-clock time.

**Model Routing** (`/route`): Route specific tools to different models.
Use small models for grep/ls, large models for code generation.

**Renderer** (`render.rs`): Trait for output rendering. `TerminalRenderer`
handles standard output. `KidsRenderer` wraps it with child-friendly
messages (hides JSON, exit codes, file paths). `select_renderer()` picks
the implementation based on kids mode. Future renderers add image display
(Kitty/Sixel), web UI, etc.

**TurnPolicy** (`interactive.rs`): Per-turn behavioral decisions derived
from `Agent::is_kids_mode()`. Captures auto-approve, rate limiting, and
renderer selection in one struct instead of scattered `if is_kids` checks.

**KidsSandbox** (`executor.rs`): Restricts workspace path and shell
commands when kids mode is active. Two-layer validation: command allowlist
+ interpreter file validation (blocks `-c`/`-e` inline code, verifies
script paths are within the sandbox workspace).

**Launch Profiles** (`settings.rs`): `[[profiles]]` in config.toml bundle
persona + mode + skills + model into `anvil --profile <name>`. Last-used
profile remembered across sessions.

**Project Detection** (`system_prompt.rs`): Auto-detects Rust, Node.js,
Python, Go, Docker from workspace files. Injected into system prompt so
the model knows the project type without being told.

**Event** (`event.rs`): Source-agnostic trigger enum (`UserPrompt`,
`FileChanged`, `Shutdown`). Decouples prompt source from agent logic.
In v2.0, adding a UDS listener means adding a new event producer —
zero changes to dispatch or agent code.

**SessionSnapshot** (`session.rs`): Persists agent state (mode, persona,
skills, profile) to SQLite after every turn. `anvil --continue` restores
the full agent state, not just messages. Bridge to v2.0 daemon resume.

### Harness directory

```
.anvil/
├── config.toml          # Provider, agent, tool, MCP settings
├── context.md           # Injected into system prompt
├── inventory.toml       # Host/service registry (optional)
├── achievements.json    # Unlocked badges
├── models/              # Per-model sampling profiles (TOML)
├── skills/              # 22 bundled skills (Markdown + YAML frontmatter)
├── layouts/             # 3 bundled Zellij layouts (KDL)
└── memory/              # Persistent learned patterns (categorized markdown)
```

---

## 3. Conventions & Workflow

### Rust style
- `anyhow::Result` everywhere, `bail!()` for early returns, no `unwrap()` in prod
- `tokio` async for I/O, sync for pure computation
- `serde` derive: JSON wire format, TOML config, YAML skill frontmatter
- snake_case functions, CamelCase types, SCREAMING_SNAKE constants
- `///` on all public items, `//!` for module docs — explain "why" not "what"

### Testing
- Unit tests in `#[cfg(test)] mod tests` at bottom of file
- Integration tests in `crates/*/tests/`
- `tempfile::TempDir` for filesystem tests
- Assert specific values, not just `is_some()`

### Build
```bash
cargo build                          # debug
cargo test                           # all tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

---

## 4. Lessons Learned

These prevent real bugs. Don't violate them.

- **Shell = string, not argv**: LLMs generate `ls -la /tmp`, not arrays. Shell tool uses `sh -c`.
- **Readline over TUI**: A ratatui TUI was built and deleted. Readline is simpler and more reliable.
- **Option\<Agent\> for async**: Take/put pattern solves ownership when spawning tasks.
- **Tail-truncation**: Keep the tail (recent output), not the head. Full output saved to temp file.
- **Retryable vs Permanent**: 404 is permanent, 429 is retryable. `RetryError` enum enforces this.
- **env_clear() + allowlist**: Shell uses `env_clear()`, skills declare env vars for passthrough.
- **Ollama 2048 default**: Set `OLLAMA_NUM_CTX` or use model profile `context.default_window`.
- **GLM-4.7-Flash**: Use llama-server with `--jinja`. Set `repeat_penalty = 1.0`.
- **Don't assume model capabilities**: Test with smallest model you support.
- **No DynTool trait**: A trait-based tool system was built and deleted. Simple match dispatch is enough.
- **tool_choice is required**: Without `tool_choice: "auto"` in the API request, models may ignore tools and print code inline instead of using `file_write`.
- **Modes over auto-detection**: Explicit `/mode coding|creative` is simpler and more reliable than trying to auto-detect intent from the user's prompt.
- **Profiles over manual setup**: Kids can't type `/persona sparkle` + `/mode creative` + `/skill cool-stuff`. One `anvil -p sparkle` flag does everything.
- **Project detection is lightweight**: Only check file existence, don't parse contents. The model needs a hint ("Rust project"), not a full analysis.
- **tool_choice fallback for MLX**: MLX rejects `tool_choice` with 400/422. Client retries once without it. Don't fail the whole request over a hint parameter.
- **BYOB over process management**: Anvil is a CLI agent, not a process supervisor. Use Zellij layouts to manage backend lifecycle. `backend.rs` exists for optional managed backends but Zellij is the preferred approach.
- **Static context over auto-sizing**: Model profiles declare `recommended_context` statically. No GGUF parsing, no memory math. Trust the profile.
- **Event enum over trait dispatch**: `Event` is an enum, not `Box<dyn EventSource>`. Compiler verifies exhaustiveness. Adding a v2.0 UDS variant is one match arm, not a trait implementation.
- **Snapshot metadata, not message re-serialization**: `SessionSnapshot` stores mode/persona/skills/profile — not messages. Messages are already saved individually during the turn. Don't serialize the same data twice.
- **KV cache is the backend's problem**: On session resume, Anvil sends full message history. The backend decides whether to recompute or reuse cache. Don't try to detect cache state from the client side.
- **Workspace-scoped sockets**: Hash the workspace path into the socket filename. Multiple daemons can run concurrently in different projects without collision.
- **Mtime ledger over inotify cookies**: Track agent writes by recording `(path, mtime)` after `file_write`/`file_edit`. The watcher checks mtime match — if it matches, it's our write. Simpler than trying to correlate inotify event IDs.
- **Timeout over backpressure**: Wrap IPC writes in a 3-second timeout. A slow client should be shed, not allowed to block the agent dispatch loop.
- **tool_choice=required for action-first personas**: Small models with `tool_choice: auto` often converse instead of executing tools. Kids personas force `tool_choice: required` via `Agent::is_kids_mode()`.
- **KidsRenderer over inline conditionals**: Kids-specific rendering (fun messages, metadata stripping) belongs in a `KidsRenderer` that wraps `TerminalRenderer`, not in `if is_kids` branches scattered through `interactive.rs`. The interactive loop calls the `Renderer` trait uniformly.
- **Per-profile base_url**: Different profiles can point to different backend servers. Kids on `:8081`, coding on `:8080`. `LaunchProfile.base_url` overrides `ProviderConfig.base_url`.
- **Sandbox interpreters need file validation**: Allowing `python3` in the kids command allowlist isn't enough — `python3 -c "os.system('rm -rf /')"` bypasses it. Interpreters must run files within the sandbox workspace, not inline code.
- **TurnPolicy over scattered booleans**: Per-turn behavioral decisions (auto-approve, rate limiting, renderer) belong in a `TurnPolicy` struct derived once from `Agent::is_kids_mode()`, not in `if is_kids` checks scattered through the loop. Adding a new policy dimension is one field, not a grep-and-patch.
- **Zellij pane integration (future)**: Terminal output limitations (stack traces, multi-file diffs) would benefit from Zellij pane control — sending artifacts to split panes while keeping the chat loop clean. Not implemented; documented as a v3.0 direction.

---

## 5. Checklist

Before any change:

1. Does `cargo test` pass before and after?
2. Works on macOS AND Linux? Use `#[cfg(unix)]`, not `#[cfg(target_os = "macos")]`.
3. Simplest solution? Prefer boring over clever.
4. Respects crate boundaries? `anvil-config` has no internal deps.
5. Works with all backends? Test against at least two.
6. Does the LLM actually generate this format? Test with a real model.
7. Adding a dependency? Check workspace `Cargo.toml` first.
8. Error case handled? `anyhow::Result` + `bail!()`.
9. Doc comment explains "why"?
10. Works for first-time `anvil init` user?

---

## 6. Key Files

| File | Purpose |
|------|---------|
| `crates/anvil/src/main.rs` | CLI entry, clap args, MCP init, Ralph Loop |
| `crates/anvil/src/commands.rs` | 17 slash commands (including /mode, /selftest) |
| `crates/anvil/src/interactive.rs` | Readline loop, TurnPolicy, streaming display, status line |
| `crates/anvil/src/render.rs` | Renderer trait, TerminalRenderer, KidsRenderer, select_renderer |
| `crates/anvil/src/backend.rs` | Optional managed backend process (start/stop/health-check) |
| `crates/anvil/src/watcher.rs` | File watcher (notify crate), debounce, noise filtering |
| `crates/anvil/src/ipc.rs` | IPC wire protocol: length-prefixed JSON, Request/Response enums |
| `crates/anvil/src/daemon.rs` | Daemon server: UDS listener, DaemonTask queue, dispatch loop |
| `crates/anvil/src/client.rs` | IPC client: send prompt, daemon status/stop |
| `crates/anvil-agent/src/agent.rs` | Agent::turn() core loop, is_kids_mode(), mode-aware tool_choice |
| `crates/anvil-agent/src/mode.rs` | Mode enum (Coding, Creative) |
| `crates/anvil-agent/src/skills.rs` | Skill parsing, YAML frontmatter |
| `crates/anvil-agent/src/autonomous.rs` | Ralph Loop runner |
| `crates/anvil-agent/src/achievements.rs` | Badge system, session tracker |
| `crates/anvil-agent/src/persona.rs` | 4 personas (sparkle, bolt, codebeard, homelab) |
| `crates/anvil-agent/src/system_prompt.rs` | Layered prompt builder with tool-use guidance, devcontainer detection |
| `crates/anvil-agent/src/event.rs` | Source-agnostic Event enum (v2.0 bridge) |
| `crates/anvil-agent/src/dispatch.rs` | Event dispatch — routes events to agent.turn() |
| `crates/anvil-agent/src/session.rs` | SessionStore (SQLite), SessionSnapshot, migrations 001–003 |
| `crates/anvil-agent/src/routing.rs` | Model routing — route specific tools to different models |
| `crates/anvil-agent/src/memory.rs` | Persistent learned patterns (categorized markdown in `.anvil/memory/`) |
| `crates/anvil-agent/src/thinking.rs` | ThinkingFilter — parse `<thinking>` blocks from streaming output |
| `crates/anvil-agent/src/json_filter.rs` | Extract JSON from model output (handles persona bleed) |
| `crates/anvil-config/src/profiles.rs` | 12 model profiles with capability tags and KV cache config |
| `crates/anvil-config/src/bundled_skills.rs` | 22 bundled skills |
| `crates/anvil-config/src/bundled_layouts.rs` | 3 bundled Zellij layouts (TQ, dev, ops) |
| `crates/anvil-config/src/inventory.rs` | Host/service inventory with deployment support |
| `crates/anvil-config/src/settings.rs` | Settings struct, launch profiles, MCP config |
| `crates/anvil-config/src/provider.rs` | BackendKind enum (Ollama, LlamaServer, Mlx, Custom), ProviderConfig |
| `crates/anvil-llm/src/client.rs` | LlmClient, streaming, retry, tool_choice fallback |
| `crates/anvil-llm/src/message.rs` | ChatMessage, ToolCall, ToolChoice, ChatRequest |
| `crates/anvil-llm/src/stream.rs` | SSE stream parser for chunked LLM responses |
| `crates/anvil-tools/src/tools.rs` | 11 tool implementations |
| `crates/anvil-tools/src/executor.rs` | Tool dispatch, validation, KidsSandbox, WriteLedger integration |
| `crates/anvil-tools/src/definitions.rs` | Tool JSON schema definitions for the LLM |
| `crates/anvil-tools/src/hooks.rs` | Pre/post hooks, platform-agnostic script discovery |
| `crates/anvil-tools/src/permission.rs` | Tool permission system (auto-approve, prompt, deny) |
| `crates/anvil-tools/src/truncation.rs` | Tail-truncation of tool output, temp file for full content |
| `crates/anvil-tools/src/ledger.rs` | WriteLedger: mtime tracking to prevent watcher feedback loops |
| `crates/anvil-mcp/src/manager.rs` | MCP server lifecycle, tool namespacing |

## Test Inventory

340 tests across all crates. Run with `cargo test`.

| Crate | Tests | Notes |
|-------|-------|-------|
| `anvil` (binary) | 34 | IPC, daemon, watcher, renderer (incl. KidsRenderer), backend, commands |
| `anvil-agent` | 136 | Agent loop, session store, events, dispatch, skills, personas, routing, memory, thinking. 3 env-dependent failures (devcontainer detection — pass on bare metal, fail inside containers) |
| `anvil-tools` | 84 | 27 unit + 2 definition + 55 integration (tool execution, sandbox hardening) |
| `anvil-config` | 54 | Settings, profiles (incl. per-profile base_url), skills, layouts, inventory |
| `anvil-llm` | 22 | 14 unit + 8 integration (streaming) |
| `anvil-mcp` | 10 | Client, types, JSON-RPC |

## Known Issues

1. Ollama defaults to 2048 context — set `OLLAMA_NUM_CTX` or use model profile
2. MLX tool calling varies by model — `tool_choice` auto-stripped on 400/422
3. GLM-4.7-Flash has chat template bugs on Ollama — use llama-server with `--jinja`
4. 3 tests in `anvil-agent` fail inside devcontainers (`.dockerenv` detected before other signals) — pass on bare metal
