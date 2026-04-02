# AGENTS.md — Anvil

Single source of truth for AI agents working in this codebase.

---

## 1. Identity

**Anvil** — terminal coding agent in Rust. Connects to local LLM backends
(Ollama, llama-server, MLX) via OpenAI-compatible API. Offline-first, airgapped.

- **Repo**: https://github.com/baalho/anvil-tui
- **License**: Apache-2.0
- **Rust**: edition 2021, MSRV 1.75
- **Version**: 1.7.0
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
| `anvil-config` | Settings, `.anvil/` harness, model profiles, bundled skills, inventory |
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
handles text. Future renderers add image display (Kitty/Sixel), web UI, etc.

**Launch Profiles** (`settings.rs`): `[[profiles]]` in config.toml bundle
persona + mode + skills + model into `anvil --profile <name>`. Last-used
profile remembered across sessions.

**Project Detection** (`system_prompt.rs`): Auto-detects Rust, Node.js,
Python, Go, Docker from workspace files. Injected into system prompt so
the model knows the project type without being told.

### Harness directory

```
.anvil/
├── config.toml          # Provider, agent, tool, MCP settings
├── context.md           # Injected into system prompt
├── inventory.toml       # Host/service registry (optional)
├── achievements.json    # Unlocked badges
├── models/              # Per-model sampling profiles (TOML)
├── skills/              # 21 bundled skills (Markdown + YAML frontmatter)
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
| `crates/anvil/src/interactive.rs` | Readline loop, streaming display, status line |
| `crates/anvil/src/render.rs` | Renderer trait, TerminalRenderer |
| `crates/anvil-agent/src/agent.rs` | Agent::turn() core loop, mode-aware tool_choice |
| `crates/anvil-agent/src/mode.rs` | Mode enum (Coding, Creative) |
| `crates/anvil-agent/src/skills.rs` | Skill parsing, YAML frontmatter |
| `crates/anvil-agent/src/autonomous.rs` | Ralph Loop runner |
| `crates/anvil-agent/src/achievements.rs` | Badge system, session tracker |
| `crates/anvil-agent/src/persona.rs` | 4 personas (sparkle, bolt, codebeard, homelab) |
| `crates/anvil-agent/src/system_prompt.rs` | Layered prompt builder with tool-use guidance |
| `crates/anvil-config/src/profiles.rs` | 10 model profiles with capability tags |
| `crates/anvil-config/src/bundled_skills.rs` | 21 bundled skills |
| `crates/anvil-config/src/inventory.rs` | Host/service inventory |
| `crates/anvil-config/src/settings.rs` | Settings struct, MCP config |
| `crates/anvil-llm/src/client.rs` | LlmClient, streaming, retry, tool_choice fallback |
| `crates/anvil-tools/src/tools.rs` | 11 tool implementations |
| `crates/anvil-tools/src/executor.rs` | Tool dispatch, validation |
| `crates/anvil-tools/src/hooks.rs` | Pre/post hooks, platform-agnostic script discovery |
| `crates/anvil-mcp/src/manager.rs` | MCP server lifecycle |

## Known Issues

1. Ollama defaults to 2048 context — set `OLLAMA_NUM_CTX` or use model profile
2. MLX tool calling varies by model — `tool_choice` auto-stripped on 400/422
3. GLM-4.7-Flash has chat template bugs on Ollama — use llama-server with `--jinja`
