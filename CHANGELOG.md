# Changelog

All notable changes to Anvil are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
