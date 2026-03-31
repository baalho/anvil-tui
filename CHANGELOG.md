# Changelog

All notable changes to Anvil are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [v1.0.0] — TBD

### Added
- Soak test validation for daily-driver readiness
- Config migration from older versions
- Offline documentation bundled with binary

## [v0.9.0] — TBD

### Added
- `CONTRIBUTING.md` with build setup and PR process
- Issue templates for bugs and feature requests
- PR template with checklist
- `CHANGELOG.md` in Keep a Changelog format
- `scripts/install.sh` for macOS and Linux

## [v0.8.0] — TBD

### Added
- CI on Linux (Ubuntu), macOS ARM (macos-14), and Windows
- Release workflow triggered by git tags
- Release artifacts for all three platforms

### Changed
- CI uses `macos-14` (ARM) instead of `macos-latest` (Intel)

## [v0.7.0] — TBD

### Added
- File content cache in `ToolExecutor` (avoids redundant reads)
- Cache invalidation on `file_write` and `file_edit`
- `ToolOutputDelta` agent event for streaming tool output
- Release profile: `lto = true`, `strip = true`, `codegen-units = 1`, `opt-level = "z"`

## [v0.6.0] — TBD

### Added
- Custom tool plugins via `.anvil/tools/*.toml`
- Template variable substitution (`{{arg_name}}`) in plugin commands
- Boolean conditional blocks (`{{#flag}}text{{/flag}}`)
- Skill dependency resolution (`depends: [docker]` in frontmatter)
- Transitive dependency resolution with circular dependency detection
- Hook system: `pre-shell.sh`, `post-edit.sh` scripts in `.anvil/hooks/`
- Configurable `block_on_failure` for hooks

## [v0.5.0] — TBD

### Added
- `StreamEvent::Error` for mid-stream backend disconnection notification
- Input validation layer in `ToolExecutor` with actionable error messages
- SIGTERM → SIGKILL timeout escalation for shell commands (Unix)
- Partial output capture on shell timeout

### Changed
- Shell timeout returns `Ok` with error message (not `Err`) so LLM can self-correct

## [v0.4.0] — TBD

### Added
- Model routing: `/route shell qwen3:8b` sends tool calls to specific models
- `ModelRouter` with per-tool and wildcard (`*`) routes
- Parallel tool execution: read-only tools run concurrently via `tokio::spawn`
- `ToolExecutor` is now `Clone` (uses `Arc<PermissionHandler>`)
- Per-session cost tracking persisted in SQLite
- Enhanced `/stats`: shows routes, thinking mode, request count, local cost indicator

## [v0.3.0] — TBD

### Added
- Sliding window context management
- Full-text session search via FTS5 (`anvil history --search`)
- Project memory (`.anvil/memory/*.md`) injected into system prompt
- `/memory add` and `/memory clear` commands

## [v0.2.0] — TBD

### Added
- Interactive Ralph Loop (`/ralph --verify "cmd" --max-iterations N`)
- Backend lifecycle management (`/backend start`, `/backend stop`)
- Model switching with profile auto-apply (`/model qwen3:8b`)
- `/think` command to toggle thinking block visibility

## [v0.1.1] — TBD

### Added
- Ctrl+C cancellation of in-flight LLM requests via `CancellationToken`
- `ThinkingFilter` state machine for `<think>` block parsing
- Context compaction via LLM-based summarization (`/compact`)
- Auto-compaction when context exceeds configurable threshold
- `AgentEvent::ThinkingDelta`, `Cancelled`, `AutoCompacted` variants

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
