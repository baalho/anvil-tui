# Anvil

A terminal coding agent forged in Rust. Connects to local models via Ollama,
llama-server, or MLX. Runs offline. Works airgapped. Version 3.0.

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

See [MANUAL.md](MANUAL.md) for TurboQuant (262K context), MLX, and
daemon setup.

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

## Daemon Mode

Run Anvil as a background server. Send prompts from any terminal.
Each workspace gets its own daemon socket — multiple projects run concurrently.

```bash
# Start the daemon
anvil daemon start

# Send prompts from anywhere
anvil send "explain the auth module"
anvil send -y "fix the failing test"

# Pipe-friendly: stdout is content, stderr is diagnostics
anvil send "list all TODO comments" > todos.txt

# Check status / stop
anvil daemon status
anvil daemon stop
```

## Watch Mode

Monitor your workspace for file changes and react automatically.
Agent's own file writes are suppressed via mtime ledger to prevent feedback loops.

```bash
anvil watch                          # watch with 2s debounce
anvil watch --debounce 5             # custom debounce
anvil watch --ignore vendor/         # ignore patterns
```

## TurboQuant (262K Context)

Run a 30B model with 262K context on a 64GB MacBook:

```bash
# Build turboquant_plus (llama.cpp fork with Metal TQ kernels)
git clone https://github.com/TheTom/turboquant_plus.git
cd turboquant_plus
cmake -B build -DGGML_METAL=ON -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release -j$(sysctl -n hw.ncpu)

# Launch with TurboQuant
./build/bin/llama-server \
  -m ~/models/qwen3-coder-30b-a3b-q4_k_m.gguf \
  --cache-type-k q8_0 --cache-type-v turbo4 \
  --jinja -ngl 99 -c 262144 -fa on --port 8080

# Configure Anvil
# .anvil/config.toml:
#   [provider]
#   backend = "llama-server"
#   base_url = "http://localhost:8080/v1"
#   model = "qwen3-coder-tq4"
```

See [MANUAL.md — TurboQuant Setup](MANUAL.md#turboquant-setup) for
the full walkthrough.

## Features

### Multi-Backend Support
- **Ollama** — easy setup, auto-pulls models
- **llama-server** — TurboQuant, best chat template fidelity
- **MLX** — Apple Silicon native inference

### 22 Bundled Skills

| Category | Skills |
|----------|--------|
| Infrastructure | containers, server-admin, sops-age, deploy-fish, tailscale, caddy-cloudflare, restic-backup, grafana, prometheus, deploy |
| Dev Tools | nvim, zellij, fish, git-workflow |
| Meta | verify-all, verify-shell, verify-files, learn-anvil, learn-rust |
| Kids | kids-first, kids-story, kids-game |

### 11 Built-in Tools
`shell`, `file_read`, `file_write`, `file_edit`, `grep`, `ls`, `find`,
`git_status`, `git_diff`, `git_log`, `git_commit`

### Model Profiles
Per-model sampling parameters with TurboQuant KV cache config.
12 bundled profiles including TQ4 (262K) and TQ3 (512K).

### Autonomous Mode (Ralph Loop)
Retry until a verification command passes:
```bash
anvil run -p "fix all tests" -a --verify "cargo test" --max-iterations 5
```

### Session Persistence
SQLite-backed sessions with incremental crash recovery. Resume with
`anvil -c`. Daemon mode preserves state across restarts.

### Launch Profiles
Bundle persona + mode + skills + model into one flag:
```bash
anvil -p tq          # TurboQuant coding setup
anvil -p sparkle     # Kids mode
```

### MCP (Model Context Protocol)
Connect external tool servers via MCP over stdio.

### Character Personas
Fun mode for kids: `/persona sparkle` activates Sparkle the Coding Unicorn.
Also: Bolt the Robot, Captain Codebeard, Homelab Admin.

## Interactive Commands

```
/help                          Show all commands
/stats                         Token usage, model, backend info
/model [name]                  Show or switch model
/backend [type url]            Show or switch backend
/skill [name]                  List, activate, or verify skills
/ralph <prompt> --verify <cmd> Autonomous mode
/clear                         Compact conversation context
/think                         Toggle <think> block visibility
/route [tool model]            Show or set model routing
/memory                        List/add/search stored patterns
/mcp                           List MCP servers and tools
/persona [name]                Activate a character persona
/mode [coding|creative]        Switch operating mode
/selftest                      Run self-diagnostics
/inventory                     Show host/service inventory
/history                       List recent sessions
/end                           End session and exit
```

## Documentation

- [MANUAL.md](MANUAL.md) — full setup guide (TurboQuant, MLX, daemon, watch)
- [CHANGELOG.md](CHANGELOG.md) — version history
- [AGENTS.md](AGENTS.md) — AI agent conventions

## Project Structure

```
crates/
+-- anvil-config    # Settings, model profiles, bundled skills/layouts
+-- anvil-llm       # OpenAI-compatible HTTP client, SSE streaming, retry
+-- anvil-tools     # 11 tools, executor, permissions, output truncation
+-- anvil-mcp       # MCP client — JSON-RPC over stdio
+-- anvil-agent     # Agent loop, Event enum, dispatch, sessions, skills
+-- anvil           # CLI binary, daemon, watch, IPC client
```

## License

Apache-2.0
