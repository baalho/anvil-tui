# AGILE.md — Anvil Project Plan

Feature-driven milestones from v0.1.1 to v1.0. Ship when ready, not on a
calendar. Each milestone has a theme, user stories, and acceptance criteria.

**Version scheme**: `v0.MINOR.PATCH`. Minor = feature milestone. Patch = bug
fixes within a milestone. v1.0 = production-ready daily driver.

---

## v0.1.1 — Stability

Theme: Fix rough edges discovered during initial use.

### User Stories

**S1: Ctrl+C cancellation**
As a user, I want to press Ctrl+C to cancel an in-flight LLM request,
so that I don't have to wait for a slow response or kill the process.

- Acceptance: Ctrl+C during streaming stops the request and returns to the prompt.
- Acceptance: Ctrl+C during tool execution cancels the command.
- Acceptance: Double Ctrl+C exits Anvil entirely.

**S2: Thinking mode parsing**
As a user, I want `<think>` blocks from Qwen3 and DeepSeek-R1 to be hidden
or collapsed, so that the output is clean and readable.

- Acceptance: `<think>...</think>` blocks are stripped from displayed output.
- Acceptance: Thinking content is still stored in session history.
- Acceptance: A `/think` toggle command shows/hides thinking blocks.

**S3: Context compaction**
As a user, I want `/clear` to compact the conversation context,
so that I can continue working without hitting the context window limit.

- Acceptance: `/clear` summarizes the conversation and replaces old messages.
- Acceptance: Tool call history is preserved in summary form.
- Acceptance: Token count drops after compaction.

### Definition of Done

- All three stories implemented and tested.
- Existing 91 tests still pass.
- No new clippy warnings.

---

## v0.2.0 — Interactive Autonomy

Theme: Bring autonomous mode into the interactive session.

### User Stories

**S4: Interactive Ralph Loop**
As a user, I want to run `/ralph "fix tests" --verify "cargo test"` from
the interactive prompt, so that I don't have to exit and use `anvil run -a`.

- Acceptance: `/ralph <prompt> --verify <cmd>` starts an autonomous loop.
- Acceptance: Progress is displayed (iteration count, verify result).
- Acceptance: Ctrl+C stops the loop and returns to the prompt.
- Acceptance: Results are stored in the current session.

**S5: Backend lifecycle management**
As a user, I want Anvil to start and stop llama-server automatically,
so that I don't have to manage backend processes manually.

- Acceptance: `anvil --backend llama --model <path.gguf>` starts llama-server.
- Acceptance: Anvil stops llama-server on exit.
- Acceptance: If llama-server is already running, Anvil connects to it.

**S6: Model switching with profile reload**
As a user, I want `/model <name>` to also reload the matching model profile,
so that sampling parameters update when I switch models.

- Acceptance: `/model devstral` applies Devstral's sampling config.
- Acceptance: `/model` with no args shows current model and active profile.
- Acceptance: Switching to a model with no profile clears sampling overrides.

### Definition of Done

- All three stories implemented and tested.
- Interactive Ralph Loop works with Ctrl+C cancellation.
- Backend lifecycle tested with llama-server.

---

## v0.3.0 — Context Intelligence

Theme: Smarter context management and memory.

### User Stories

**S7: Sliding window context**
As a user, I want Anvil to automatically manage context when approaching
the limit, so that I don't have to manually run `/clear`.

- Acceptance: When context exceeds 90% of the window, oldest messages are
  summarized and replaced automatically.
- Acceptance: System prompt and most recent N messages are always preserved.
- Acceptance: A notification is shown when auto-compaction occurs.

**S8: Session search**
As a user, I want to search past sessions by content,
so that I can find and resume relevant work.

- Acceptance: `anvil history --search "docker"` finds sessions mentioning docker.
- Acceptance: Search covers user messages, assistant responses, and tool outputs.

**S9: Project memory**
As a user, I want Anvil to remember project-specific patterns across sessions,
so that it doesn't repeat mistakes.

- Acceptance: Anvil writes learned patterns to `.anvil/memory/`.
- Acceptance: Memory is loaded into context on session start.
- Acceptance: `/memory` command lists and manages stored patterns.

### Definition of Done

- Sliding window works without losing important context.
- Session search returns relevant results.
- Memory persists across sessions.

---

## v0.4.0 — Multi-Model

Theme: Use different models for different tasks within one session.

### User Stories

**S10: Model routing**
As a user, I want to route specific tasks to specific models,
so that I can use a fast model for simple tasks and a capable model for complex ones.

- Acceptance: `/route shell qwen3:8b` routes shell-related tasks to a smaller model.
- Acceptance: Default model handles unrouted tasks.
- Acceptance: Routing rules persist in `.anvil/config.toml`.

**S11: Parallel tool execution**
As a user, I want independent tool calls to execute in parallel,
so that multi-file operations are faster.

- Acceptance: When the LLM returns multiple tool calls with no dependencies,
  they execute concurrently.
- Acceptance: Results are collected and sent back in order.
- Acceptance: Errors in one tool don't block others.

**S12: Cost tracking**
As a user, I want to see estimated costs when using paid API endpoints,
so that I can monitor spending.

- Acceptance: `/stats` shows estimated cost when pricing is configured.
- Acceptance: Cost is calculated from `PricingConfig` in settings.
- Acceptance: Local models show $0.00.

### Definition of Done

- Model routing works with at least two models simultaneously.
- Parallel execution measurably faster for multi-tool turns.
- Cost tracking accurate for configured pricing.

---

## v0.5.0 — Resilience

Theme: Handle failures gracefully.

### User Stories

**S13: Graceful error recovery**
As a user, I want Anvil to recover from backend disconnections,
so that a temporary network issue doesn't lose my session.

- Acceptance: If the backend goes down mid-stream, Anvil retries after reconnection.
- Acceptance: Session state is preserved during disconnection.
- Acceptance: User is notified of the disconnection and recovery.

**S14: Tool timeout handling**
As a user, I want shell commands that exceed the timeout to be killed cleanly,
so that hung processes don't block the agent.

- Acceptance: Shell commands exceeding `shell_timeout_secs` are killed with SIGTERM.
- Acceptance: If SIGTERM doesn't work, SIGKILL after 5 seconds.
- Acceptance: Timeout is reported to the LLM as an error with partial output.

**S15: Input validation**
As a user, I want Anvil to validate tool arguments before execution,
so that malformed LLM output doesn't cause cryptic errors.

- Acceptance: Missing required arguments produce clear error messages.
- Acceptance: Invalid JSON in tool call arguments is handled gracefully.
- Acceptance: Path traversal attempts are blocked with an explanation.

### Definition of Done

- Backend disconnection recovery tested with Ollama restart.
- Timeout handling tested with `sleep 999` command.
- All validation errors produce user-friendly messages.

---

## v0.6.0 — Extensibility

Theme: Let users extend Anvil without modifying source code.

### User Stories

**S16: Custom tool plugins**
As a user, I want to define custom tools via configuration,
so that I can add project-specific capabilities without forking Anvil.

- Acceptance: Tools defined in `.anvil/tools/*.toml` with name, description,
  parameters, and a shell command template.
- Acceptance: Custom tools appear in the LLM's tool list alongside built-in tools.
- Acceptance: Custom tools respect the same permission model as built-in tools.

**S17: Skill composition**
As a user, I want skills to reference other skills,
so that I can build complex workflows from simple building blocks.

- Acceptance: `depends: [git-workflow, docker]` in frontmatter activates
  dependencies when the skill is activated.
- Acceptance: Circular dependencies are detected and reported.

**S18: Hook system**
As a user, I want to run scripts before/after tool execution,
so that I can enforce project-specific policies.

- Acceptance: `.anvil/hooks/pre-shell.sh` runs before every shell command.
- Acceptance: `.anvil/hooks/post-edit.sh` runs after every file edit.
- Acceptance: Hook failure blocks the tool execution (configurable).

### Definition of Done

- At least one custom tool plugin tested end-to-end.
- Skill composition works with 3+ levels of dependencies.
- Hooks tested with pre-shell and post-edit scenarios.

---

## v0.7.0 — Performance

Theme: Make Anvil fast for large projects.

### User Stories

**S19: Incremental context loading**
As a user, I want Anvil to load only relevant files into context,
so that large projects don't overwhelm the context window.

- Acceptance: File contents are loaded on-demand via tool calls, not upfront.
- Acceptance: Previously read files are cached in the session.

**S20: Streaming tool output**
As a user, I want to see tool output as it streams,
so that long-running commands show progress.

- Acceptance: Shell command output streams to the terminal in real-time.
- Acceptance: The LLM receives the final output after the command completes.

**S21: Binary size optimization**
As a developer, I want the release binary to be as small as practical,
so that distribution is easy.

- Acceptance: Release binary under 10MB (currently 13MB).
- Acceptance: `strip` and LTO applied in release profile.

### Definition of Done

- Incremental loading tested with a 1000+ file project.
- Streaming output visible for `cargo build` and similar commands.
- Binary size reduced with documented build flags.

---

## v0.8.0 — Cross-Platform

Theme: First-class support on all target platforms.

### User Stories

**S22: Linux CI**
As a developer, I want CI to build and test on Linux,
so that regressions are caught before merge.

- Acceptance: GitHub Actions runs `cargo test` on Ubuntu.
- Acceptance: Clippy and doc warnings fail the build.

**S23: Windows/WSL testing**
As a developer, I want CI to verify Windows/WSL compatibility,
so that WSL users have a working experience.

- Acceptance: `cargo build` succeeds on Windows (WSL).
- Acceptance: Shell tool uses `sh -c` on WSL, `cmd.exe /C` on native Windows.

**S24: macOS ARM CI**
As a developer, I want CI to build on macOS ARM,
so that Apple Silicon users get verified builds.

- Acceptance: GitHub Actions runs `cargo test` on macos-14 (ARM).
- Acceptance: Release artifacts include macOS ARM binary.

### Definition of Done

- CI green on Linux, macOS ARM, and Windows/WSL.
- Release workflow produces binaries for all three platforms.

---

## v0.9.0 — Community

Theme: Make Anvil ready for external contributors.

### User Stories

**S25: Contributing guide**
As a contributor, I want a CONTRIBUTING.md that explains how to set up,
test, and submit changes, so that I can contribute without guessing.

- Acceptance: CONTRIBUTING.md covers build setup, test commands, PR process.
- Acceptance: Issue templates for bugs and feature requests.

**S26: Changelog**
As a user, I want a CHANGELOG.md that lists changes per version,
so that I know what's new when I update.

- Acceptance: CHANGELOG.md follows Keep a Changelog format.
- Acceptance: Updated with every version bump.

**S27: Install script**
As a user, I want a one-line install command,
so that I can get started without cloning and building.

- Acceptance: `curl -sSL https://anvil.dev/install.sh | sh` works on macOS and Linux.
- Acceptance: Installs pre-built binary to `~/.local/bin/`.
- Acceptance: Verifies checksum.

### Definition of Done

- A new contributor can go from zero to running tests in under 5 minutes.
- CHANGELOG covers all versions from v0.1.0.
- Install script tested on macOS and Ubuntu.

---

## v1.0.0 — Daily Driver

Theme: Production-ready for daily use.

### User Stories

**S28: Stability guarantee**
As a user, I want Anvil to run for hours without crashes or memory leaks,
so that I can trust it for long coding sessions.

- Acceptance: 8-hour soak test with continuous prompts passes.
- Acceptance: Memory usage stays stable (no unbounded growth).
- Acceptance: All error paths tested with fuzzing or property tests.

**S29: Configuration migration**
As a user, I want Anvil to migrate old config formats automatically,
so that updates don't break my setup.

- Acceptance: Config format changes include migration logic.
- Acceptance: Old `.anvil/config.toml` files are upgraded in place.
- Acceptance: Migration is logged so users know what changed.

**S30: Offline documentation**
As a user, I want `anvil help <topic>` to show built-in documentation,
so that I can get help without internet access.

- Acceptance: `anvil help tools` lists all tools with descriptions.
- Acceptance: `anvil help skills` explains the skills system.
- Acceptance: `anvil help config` shows all configuration options.

### Definition of Done

- All v0.x features stable and tested.
- No known crashes or data loss bugs.
- Documentation complete (README, MANUAL, CONTRIBUTING, CHANGELOG, inline help).
- Binary available for macOS ARM, Linux x86_64, Windows/WSL.

---

## Backlog

Ideas that don't fit a milestone yet. Pull into a milestone when prioritized.

- **Vision support**: Pass screenshots to multimodal models
- **MCP integration**: Connect to Model Context Protocol servers
- **LSP integration**: Use language server for smarter code understanding
- **Git integration**: Built-in diff, commit, PR creation tools
- **Web search tool**: Search the web for documentation and examples
- **REPL mode**: Execute code snippets in a persistent REPL
- **Team sharing**: Share skills and profiles via git-hosted registries
- **Telemetry (opt-in)**: Anonymous usage stats for prioritizing features
- **Conversation branching**: Fork a conversation to explore alternatives
- **Multi-file diff view**: Show all changes in a unified diff before applying
