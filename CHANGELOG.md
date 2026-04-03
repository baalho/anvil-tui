# Changelog

All notable changes to Anvil are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [v1.9.0] — 2025-07-17 — Bridge to v2.0

### Added
- **Session snapshots** — `SessionSnapshot` persists agent state (mode,
  persona, active skills, model profile) to SQLite after every turn.
  `anvil --continue` now restores the full agent state, not just messages.
  New columns added via idempotent migration.
- **Event abstraction** — `Event` enum (`UserPrompt`, `FileChanged`,
  `Shutdown`) decouples trigger sources from agent logic. `dispatch_event()`
  routes any event to `agent.turn()` without knowing the source. In v2.0,
  adding a UDS listener requires zero changes to dispatch or agent code.
- **`anvil watch` command** — native file watcher using the `notify` crate.
  Monitors the workspace for file changes, debounces editor save storms
  (configurable, default 2s), filters noise (`.git/`, `target/`,
  `node_modules/`, swap files, hidden files), and triggers agent turns
  automatically. Supports `--ignore` patterns and `--debounce` interval.
- **22 new tests** — session snapshot roundtrip (6), event enum (3),
  dispatch prompt formatting (3), file watcher noise filtering (10).

### Fixed
- `anvil --profile` no longer panics when `Settings` is consumed by
  `Agent::new()` — now clones settings before passing to agent.
- Skill activation in launch profiles uses `SkillLoader::scan()` instead
  of nonexistent `load_all()` method.

### Design Decision
- **Stateless autonomy over daemon** — v1.9 builds the abstractions
  (Event enum, dispatch loop, session persistence) that v2.0 needs,
  without introducing IPC, process supervision, or socket management.
  Every line of v1.9 code is load-bearing in v2.0.

## [v1.8.0] — 2025-07-17 — BYOB TurboQuant & Ops Platform

### Added
- **TurboQuant KV cache profiles** — `KvCacheConfig` struct with `type_k`,
  `type_v`, and `recommended_context` fields. Two bundled profiles for
  Qwen3-Coder with turbo4 (262K context) and turbo3 (512K context).
  `recommended_context` overrides `context.default_window` when matched.
- **Zellij layouts** — three bundled KDL layouts written to
  `.anvil/layouts/` by `anvil init`:
  - `anvil-tq.kdl` — TurboQuant (llama-server + Anvil in split panes)
  - `anvil-dev.kdl` — development (Anvil + editor + shell)
  - `anvil-ops.kdl` — homelab operations (Anvil + SSH + logs)
- **`--zellij [layout]` CLI flag** — launches Anvil inside a Zellij
  session with the named layout. Detects `$ZELLIJ` to prevent nesting.
  Skips launch inside devcontainers.
- **Devcontainer detection** — `detect_devcontainer()` checks 4
  indicators (/.dockerenv, REMOTE_CONTAINERS, CODESPACES, /workspaces/
  prefix, devcontainer.json). Injects Layer 4d into system prompt.
- **Deployment skill** — `deploy.md` bundled skill for deploying
  services to inventory hosts using SOPS/age secrets and SSH.
- **Structured deployments in inventory** — `[[hosts.deployments]]`
  with `name`, `port`, `secrets`, and `compose_file` fields. Deployment
  details rendered in system prompt with runtime-specific commands.
- **KV cache info in `/model` and startup banner** — shows cache type
  and effective context when a TQ profile is matched.
- **`ModelProfile::effective_context()`** — returns `recommended_context`
  from `[kv_cache]` if present, otherwise `context.default_window`.

### Design Decision
- **BYOB (Bring Your Own Backend)** — Anvil does not manage inference
  server processes. Zellij layouts handle backend lifecycle. This follows
  the "prefer boring over clever" principle from AGENTS.md.

## [v1.7.0] — 2025-07-17 — MLX Hardening Edition

### Added
- **MLX tool_choice fallback** — when a backend rejects `tool_choice`
  with 400/422, the client retries once without it. MLX backends that
  don't support the parameter now work without manual configuration.
- **MLX Default model profile** — bundled profile matching
  `mlx-community` / `mlx_community` patterns with appropriate sampling
  defaults (temp 0.7, top_p 0.9, 128K max context).
- **Explicit MLX model discovery** — `/models` now uses the correct
  `/v1/models` endpoint for MLX backends instead of falling through
  to the generic path.
- **Project detection edge case tests** — monorepo with multiple
  markers, `compose.yaml` variant, npm default, yarn lock file.
- **Mode integration tests** — verify Creative mode omits tools,
  Coding mode includes tools, persona auto-sets mode.
- **Capability parsing tests** — verify all bundled profiles have
  capabilities, defaults to empty, TOML parsing roundtrip.

### Changed
- **Renderer pipeline completed** — all agent output now routes through
  the `Renderer` trait: thinking blocks, tool pending/result, command
  results, compaction messages. `interactive.rs` no longer imports
  crossterm directly for these operations.
- `render_tool_result()` signature decoupled from `ToolOutput` — takes
  icon, line count, and char count instead.
- Added `render_thinking_start()`, `render_thinking_end()`,
  `render_tool_pending()` to Renderer trait.

## [v1.6.0] — 2026-04-02 — Session Awareness Edition

### Added
- **Launch profiles** (`--profile` / `-p`) — bundle persona + mode +
  skills + model into a single CLI flag. `anvil -p sparkle` starts
  with Sparkle persona, creative mode, and kids skills in one command.
  Profiles defined in `[[profiles]]` section of `.anvil/config.toml`.
- **Last profile memory** — Anvil remembers the last-used profile and
  shows a hint on next launch: "last profile: sparkle (2 hours ago)".
- **Project auto-detection** — scans workspace for Cargo.toml,
  package.json, pyproject.toml, go.mod, Makefile, Dockerfile, and
  docker-compose.yml. Injects project type into system prompt so the
  model knows the build system and test commands without being told.
- **`/selftest` command** — verifies all 8 tool categories work
  (file_write, file_read, file_edit, shell, ls, grep, find, git_status)
  without making any LLM calls. Quick health check when switching
  models or backends.
- **Conversation starters** — kids personas show 3 random suggestions
  after the greeting ("Try saying: 1. I like cats"). Typing a number
  sends the suggestion. Prevents blank-prompt freeze for young users.
- **Session summary on exit** — shows duration, tokens used, tools
  called, and files created. Kids personas get a celebratory summary
  ("You made 2 cool things!"), coding mode gets factual stats.
- **Example profiles in `anvil init`** — generated config.toml includes
  commented-out profile examples.

### Changed
- `/stats` shows mode.
- `/help` includes `/selftest` and `/mode`.
- Persona struct now includes `suggestions` field (8 per kids persona).

## [v1.5.0] — 2026-04-02 — Intent-Aware Edition

### Added
- **`tool_choice` parameter** — `ChatRequest` now sends `tool_choice`
  to the LLM API. Coding mode sends `"auto"` (model decides when to
  use tools), Creative mode sends `"none"` (model responds directly).
  This fixes the core issue where models would print code inline
  instead of using `file_write`.
- **Mode system** (`mode.rs`) — `Coding` and `Creative` modes control
  tool availability and response style. Coding mode sends all tools
  with `tool_choice: "auto"`. Creative mode omits tools entirely so
  the model responds directly (ASCII art, stories, explanations).
- **`/mode` slash command** — `/mode coding` or `/mode creative` to
  switch modes. `/mode` shows current mode. Personas auto-set mode:
  kids personas (sparkle, bolt, codebeard) → Creative, homelab → Coding.
- **Model profile capabilities** — `[capabilities]` section in model
  profiles with `strengths` tags (coding, creative, reasoning,
  tool-calling). Displayed by `/model` to help users pick the right
  model for their task.
- **Status line prompt** — prompt now shows `[mode|model]` with
  persona name when active. Mode icon: ⚒ for coding, ✨ for creative,
  persona-specific icons when a persona is active.
- **Renderer trait** (`render.rs`) — output rendering pipeline with
  `TerminalRenderer` implementation. Provides a seam for future
  renderers (image display, web UI) without touching agent logic.
- **`content_type` on ToolOutput::Structured** — hints the renderer
  about output format ("text", "image", "svg", "table"). Groundwork
  for future image generation support.
- **Tool-use guidance in system prompt** — explicit "When to Use Tools
  vs Respond Directly" section tells the model when to use `file_write`
  vs respond inline.

### Changed
- `/stats` now shows current mode.
- Banner shows mode at startup.
- All 9 bundled model profiles now include `[capabilities]` tags.

## [v1.4.0] — 2026-04-02 — Sparkle Edition

### Added
- **JSON extraction filter** (`json_filter.rs`) — regex-based extraction
  of JSON from persona-contaminated LLM output. Handles persona bleed
  where local models wrap tool call arguments in conversational text.
- **Kids sandbox** — config-driven `kids_workspace` path and shell
  command allowlist when a kids persona is active. Defense-in-depth:
  file tools resolve paths relative to the restricted workspace, and
  shell commands are checked against an allowlist before execution.
- **MCP zombie process prevention** — `McpManager` now implements
  `Drop` with `SIGKILL` cleanup. Child processes are spawned with
  `setsid()` so `kill(-pgid)` cleans up the entire process tree.
- **ToolOutput enum** — tools can return `Text` or `Structured` output.
  `Structured` carries both human-readable text (for LLM conversation)
  and machine-readable `serde_json::Value` (for future STEM rendering).
- **Input debouncing** — 2-second cooldown between user messages when a
  kids persona is active. Prevents button-mashing from flooding the
  agent with rapid-fire LLM requests.
- **System prompt layer contract test** — enforces the KV cache
  optimization ordering (static → dynamic) with a test that fails if
  layer order is violated.
- `/inventory` slash command — view hosts from `.anvil/inventory.toml`

### Changed
- `ToolExecutor::execute()` returns `ToolOutput` instead of `String`
- `AgentEvent::ToolResult` carries `ToolOutput` instead of `String`
- 15 slash commands (added `/inventory`)

## [v1.3.0] — 2026-04-02

### Added
- `/inventory` slash command — view hosts from `.anvil/inventory.toml`
- Achievements wired into interactive mode — badges unlock during sessions
- Thinking block box-drawing visualization (`╭─ thinking` / `│` / `╰─`)
- Spinner shows elapsed time (`⠋ thinking... (3s)`)
- MCP server restart (`/mcp restart <name>`)
- Ollama context warning when `OLLAMA_NUM_CTX` is not set
- E2E smoke test scripts (`scripts/test-e2e.sh`, `scripts/test-e2e.ps1`)

### Changed
- Hooks are now platform-agnostic — discovers `.sh`, `.ps1`, `.cmd`, `.bat` by platform priority
- 15 slash commands (was 14, added `/inventory`)

### Removed
- Dead TUI mode (`app.rs`, 781 lines) — was decoupled and unused
- Unused `DynTool` trait system (`tool_trait.rs`, 396 lines)
- `--tui` CLI flag and `docs` subcommand
- Help topic markdown files (`help/*.md`)

### Fixed
- `AchievementStore::load` was never called in interactive mode (only in deleted app.rs)
- MCP restart was a stub that always returned an error

## [v1.2.0] — 2026-04-02

### Added
- 6 new infrastructure skills: containers (unified Docker/Podman), sops-age, deploy-fish, tailscale, caddy-cloudflare, restic-backup
- Homelab persona (`/persona homelab`) with auto-activated infrastructure skills
- Host inventory system (`.anvil/inventory.toml`) — injected into system prompt
- Spinner animation during LLM response wait
- Categorized `/help` output with colored headers
- Colored tool output headers with per-tool icons
- Token usage display in input prompt (>50% context)

### Changed
- Unified `containers` skill replaces separate `docker` and `docker-compose` skills (21 total, was 17)
- AGENTS.md rewritten for token efficiency (~1090 tokens, was ~2957 — 63% reduction)
- Server-admin skill updated with Tailscale SSH and Podman service management
- All documentation updated with correct dependency graph, skill counts, and inventory docs
- Removed all hardware-specific references from docs and code
- Version bumped to 1.2.0 across all 6 crates

### Fixed
- AGENTS.md incorrectly stated anvil-mcp depends on anvil-config (it has no internal deps)
- MANUAL.md described memory/ as "reserved for future use" (fully implemented)
- MANUAL.md incorrectly stated anvil-tools has no internal dependencies (depends on anvil-config)

## [v1.1.0] — 2025-04-01

### Added
- MCP (Model Context Protocol) client — connect to external tool servers
- Git tools: `git_status`, `git_diff`, `git_log`, `git_commit`
- Character personas: Sparkle the Unicorn, Bolt the Robot, Captain Codebeard
- Achievement system with session tracking and unlock notifications
- Project memory (`.anvil/memory/*.md`) with categories and search
- Kids skills: first-program, storytelling, game-maker
- Model profiles for Qwen 2.5, Qwen 3.5, Nemotron Cascade 2
- Interactive model picker (`/model` shows numbered list)
- Model discovery at startup (shows available models)
- Decoupled async TUI architecture (`--tui` flag)
- Universal `DynTool` trait for extensible tool system
- STEM-ready structured output types (geometry, physics, charts)
- KV-cache-friendly layered system prompt construction
- Fun first-run onboarding with persona suggestions

### Changed
- Version bumped to 1.1.0 across all 6 crates
- System prompt reordered for KV cache efficiency
- Plugin name validation includes git tools

## [v1.0.0] — 2025-03-31

### Added
- Ctrl+C cancellation via `CancellationToken`
- `ThinkingFilter` for `<think>` block parsing
- Context compaction and auto-compaction
- Interactive Ralph Loop (`/ralph --verify`)
- Backend lifecycle management (`/backend start/stop`)
- Model switching with profile auto-apply
- Sliding window context management
- Full-text session search via FTS5
- Model routing (`/route shell qwen3:8b`)
- Parallel tool execution for read-only tools
- Per-session cost tracking
- Custom tool plugins via `.anvil/tools/*.toml`
- Skill dependency resolution with cycle detection
- Hook system (`pre-shell.sh`, `post-edit.sh`)
- Input validation with actionable error messages
- SIGTERM → SIGKILL timeout escalation
- File content cache in `ToolExecutor`
- Streaming tool output
- Release profile optimizations (LTO, strip)
- CI on Linux, macOS ARM, Windows/WSL
- Install script for macOS and Linux

## [v0.1.0] — 2025-03-30

### Added
- Interactive mode with readline-style input and streaming output
- 7 built-in tools: `shell`, `file_read`, `file_write`, `file_edit`, `grep`, `ls`, `find`
- Multi-backend support: Ollama, llama-server, MLX, Custom
- Model profiles with auto-applied sampling parameters
- Skills system with YAML frontmatter and env passthrough
- 14 bundled skills across infrastructure, dev-tools, and meta categories
- Autonomous mode (Ralph Loop) with verification-based retry
- Session persistence in SQLite with resume (`anvil -c`)
- Retry with exponential backoff (Retryable vs Permanent errors)
- Context window estimation with 80% warning
- Loop detection (hash-based, configurable limit)
- Output truncation (tail-truncation by lines/bytes, temp file fallback)
