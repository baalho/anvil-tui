# Contributing to Anvil

## Prerequisites

- Rust 1.75+ (`rustup install stable`)
- A local LLM backend (Ollama recommended: `brew install ollama`)

## Build & Test

```bash
# Clone and build
git clone https://github.com/baalho/anvil-tui.git
cd anvil-tui
cargo build

# Run all tests
cargo test

# Lint
cargo clippy --all-targets -- -D warnings

# Check docs build
cargo doc --no-deps
```

## Project Structure

```
crates/
  anvil-config/   # Settings, harness, model profiles
  anvil-llm/      # OpenAI-compatible HTTP client, SSE streaming
  anvil-tools/    # 11 built-in tools, plugins, hooks, permissions
  anvil-mcp/      # MCP (Model Context Protocol) client
  anvil-agent/    # Agent loop, skills, sessions, personas, achievements
  anvil/          # CLI binary, interactive mode, slash commands
```

Dependencies flow: `anvil-config` → `anvil-llm` / `anvil-tools` → `anvil-agent` → `anvil`.
`anvil-mcp` has no internal dependencies (connects directly to `anvil-agent`).

## Adding a Tool

1. Implement in `crates/anvil-tools/src/tools.rs`
2. Add JSON schema in `crates/anvil-tools/src/definitions.rs`
3. Add dispatch in `crates/anvil-tools/src/executor.rs`
4. Classify in `crates/anvil-tools/src/permission.rs`
5. Add tests in `crates/anvil-tools/tests/tool_tests.rs`

## Adding a Slash Command

1. Add handler in `crates/anvil/src/commands.rs`
2. Add to `match` in `handle_command()`
3. Add to `help_text()`

## Pull Request Process

1. Fork and create a feature branch
2. Ensure `cargo test` and `cargo clippy --all-targets -- -D warnings` pass
3. Follow existing code style and commit message conventions
4. Open a PR against `main`

## Code Style

- `anyhow::Result` for error handling, `bail!()` for early returns
- No `unwrap()` in production code (tests only)
- Doc comments (`///`) on all public items
- Module names match the primary type they export
- Tests in `#[cfg(test)] mod tests` at the bottom of each file

## Commit Messages

```
<scope>: <what changed>
```

Examples: `agent: add context compaction`, `tools: SIGTERM timeout handling`
