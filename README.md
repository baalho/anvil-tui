# Anvil

A terminal coding agent forged in Rust. Connects to local models via Ollama,
llama-server, or MLX. Runs offline. Works airgapped.

## Install (macOS / Apple Silicon)

```bash
# Prerequisites
brew install ollama               # LLM backend
ollama serve &                    # start in background
ollama pull qwen3-coder:30b      # 19GB — best coding agent model

# Build Anvil
git clone https://github.com/baalho/anvil-tui.git
cd anvil-tui
cargo build --release
cp target/release/anvil ~/.local/bin/
```

See [MANUAL.md](MANUAL.md) for llama-server and MLX setup.

## Quick Start

```bash
# Initialize project harness (creates .anvil/ with config, skills, model profiles)
cd your-project
anvil init

# Interactive mode
anvil

# Single prompt
anvil run -p "explain this codebase"

# Auto-approve tool calls
anvil run -p "fix the build" -y

# Autonomous mode (Ralph Loop) — retry until tests pass
anvil run -p "fix all failing tests" -a --verify "cargo test"
```

## Features

### Multi-Backend Support
Connect to any OpenAI-compatible API:
- **Ollama** — easy setup, auto-pulls models
- **llama-server** — best chat template fidelity (recommended for GLM-4.7-Flash)
- **MLX** — optimized for Apple Silicon unified memory

Switch at runtime: `/backend llama http://localhost:8080/v1`

### Model Profiles
Per-model sampling parameters in `.anvil/models/*.toml`. Bundled profiles
for Qwen3-Coder, Qwen3, Devstral, DeepSeek-R1, and GLM-4.7-Flash.
Auto-applied when the model name matches. All fit on 64GB Apple Silicon.

### Skills System
17 bundled skills across four categories:

| Category | Skills |
|----------|--------|
| Infrastructure | docker, docker-compose, server-admin, grafana, prometheus |
| Dev Tools | nvim, zellij, fish, git-workflow |
| Meta | verify-all, verify-shell, verify-files, learn-anvil, learn-rust |
| Kids | kids-first, kids-story, kids-game |

Skills support YAML frontmatter for metadata, env passthrough, and
verification commands. Write your own in `.anvil/skills/`.

### Autonomous Mode (Ralph Loop)
Retry until a verification command passes:

```bash
anvil run -p "fix all tests" -a --verify "cargo test" --max-iterations 5
```

Guardrails: iteration limit, token budget, wall-clock timeout.

### 11 Built-in Tools
`shell`, `file_read`, `file_write`, `file_edit`, `grep`, `ls`, `find`,
`git_status`, `git_diff`, `git_log`, `git_commit`

### MCP (Model Context Protocol)
Connect external tool servers via MCP over stdio. Configure in
`.anvil/config.toml` under `[mcp]`. Tools are namespaced as
`mcp_{server}_{tool}` to avoid conflicts.

### Character Personas
Fun mode for kids: `/persona sparkle` activates Sparkle the Coding Unicorn.
Also available: Bolt the Robot and Captain Codebeard.

### Achievement System
10 unlockable badges that celebrate coding milestones. Persona-themed
unlock notifications. Persisted in `.anvil/achievements.json`.

### Session Persistence
SQLite-backed sessions with resume: `anvil -c` resumes the last session.

## Interactive Commands

```
/help                          Show all commands
/stats                         Token usage, model, backend info
/model [name]                  Show or switch model
/backend [type url]            Show or switch backend
/backend start llama <model>   Start a managed llama-server
/backend stop                  Stop the managed backend
/skill [name]                  List, activate, or verify skills
/ralph <prompt> --verify <cmd> Autonomous mode
/clear                         Compact conversation context
/think                         Toggle <think> block visibility
/route [tool model]            Show or set model routing
/memory                        List stored patterns
/memory add <pattern>          Save a new pattern
/memory search <keyword>       Search memories
/mcp                           List MCP servers and tools
/persona [name]                Activate a character persona
/history                       List recent sessions
/end                           End session and exit
```

## Configuration

```toml
# .anvil/config.toml
[provider]
backend = "ollama"                     # ollama | llama-server | mlx | custom
base_url = "http://localhost:11434/v1"
model = "qwen3-coder:30b"

[agent]
context_window = 8192                  # overridden by model profile
loop_detection_limit = 10
auto_compact_threshold = 80            # auto-compact at 80% context usage

[tools]
shell_timeout_secs = 30

[mcp]
servers = []
```

## Documentation

- [MANUAL.md](MANUAL.md) — full usage guide, architecture, how-tos
- [CHANGELOG.md](CHANGELOG.md) — version history

## Project Structure

```
crates/
├── anvil-config    # Settings, model profiles, bundled skills, harness management
├── anvil-llm       # OpenAI-compatible HTTP client, SSE streaming, retry
├── anvil-tools     # 11 tools, executor, permissions, output truncation
├── anvil-mcp       # MCP client — JSON-RPC over stdio for external tool servers
├── anvil-agent     # Agent loop, skills, personas, achievements, sessions
└── anvil           # CLI binary, interactive mode, slash commands
```

## License

Apache-2.0
