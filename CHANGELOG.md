# Changelog

All notable changes to Anvil are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
