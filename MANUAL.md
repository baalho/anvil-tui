# Anvil Manual

A local-first coding agent forged in Rust. Runs offline, connects to
Ollama, llama-server, or MLX. Version 2.0.

---

## Table of Contents

1. [Quick Start (5 minutes)](#quick-start)
2. [macOS Setup — Apple Silicon](#macos-setup)
3. [TurboQuant Setup (262K+ context)](#turboquant-setup)
4. [MLX Setup (Apple Silicon native)](#mlx-setup)
5. [Daemon Mode](#daemon-mode)
6. [Watch Mode](#watch-mode)
7. [The .anvil/ Directory](#the-anvil-directory)
8. [Backends](#backends)
9. [Model Profiles](#model-profiles)
10. [Skills](#skills)
11. [Autonomous Mode (Ralph Loop)](#autonomous-mode)
12. [Interactive Commands](#interactive-commands)
13. [Configuration Reference](#configuration-reference)
14. [Architecture](#architecture)

---

## Quick Start

```bash
# 1. Install Ollama and pull a model
brew install ollama
ollama serve &
ollama pull qwen3-coder:30b

# 2. Build Anvil
git clone https://github.com/baalho/anvil-tui.git
cd anvil-tui
cargo build --release
cp target/release/anvil ~/.local/bin/

# 3. Initialize and run
cd your-project
anvil init
anvil
```

That's it. You're coding with a local AI agent. Read on for
TurboQuant (262K context), MLX, daemon mode, and advanced setup.

---

## macOS Setup

### Prerequisites

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Ollama (easiest backend to start with)
brew install ollama
ollama serve &
```

### Choosing a model

| Model | Size (Q4) | Context | Best for |
|-------|-----------|---------|----------|
| qwen3-coder:30b | 19 GB | 256K | Coding agent (recommended) |
| devstral | 14 GB | 128K | SWE-Bench tasks |
| deepseek-r1:32b | 20 GB | 131K | Reasoning, chain-of-thought |
| qwen3:8b | 5 GB | 256K | Lightweight, fast iteration |

All fit on a 64 GB M4 Max. For 32 GB machines, use qwen3:8b or
devstral with reduced context.

```bash
ollama pull qwen3-coder:30b
```

### Ollama context window

Ollama defaults to 2048 tokens — far too small for coding. Fix this:

```bash
# Option 1: environment variable
export OLLAMA_NUM_CTX=32768

# Option 2: per-model Modelfile
echo 'FROM qwen3-coder:30b
PARAMETER num_ctx 32768' | ollama create qwen3-coder-32k -f -
```

Anvil warns at startup if `OLLAMA_NUM_CTX` is not set.

### Build and install

```bash
git clone https://github.com/baalho/anvil-tui.git
cd anvil-tui
cargo build --release
cp target/release/anvil ~/.local/bin/

anvil --version
```

### Initialize a project

```bash
cd your-project
anvil init
```

This creates `.anvil/` with config, model profiles, skills, and
Zellij layouts. See [The .anvil/ Directory](#the-anvil-directory).

---

## TurboQuant Setup

TurboQuant compresses the KV cache during inference, allowing
dramatically larger context windows on the same hardware. A 30B Q4
model that normally fits 32K context can run at 262K with turbo4.

### What you need

- **MacBook Pro M4 Max** (or any Apple Silicon with 64+ GB unified memory)
- **TheTom/turboquant_plus** — a llama.cpp fork with Metal-accelerated
  TurboQuant kernels
- **A GGUF model** — e.g. Qwen3-Coder-30B-Q4_K_M

### Step 1: Build turboquant_plus

```bash
# Clone the TurboQuant fork of llama.cpp
git clone https://github.com/TheTom/turboquant_plus.git
cd turboquant_plus

# Build with Metal support (Apple Silicon GPU)
cmake -B build -DGGML_METAL=ON -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release -j$(sysctl -n hw.ncpu)

# Verify the binary exists
ls build/bin/llama-server
```

### Step 2: Download a GGUF model

```bash
mkdir -p ~/models

# Download from Hugging Face (example: Qwen3-Coder 30B Q4)
# Use huggingface-cli or wget:
pip install huggingface-hub
huggingface-cli download Qwen/Qwen3-Coder-30B-A3B-GGUF \
  qwen3-coder-30b-a3b-q4_k_m.gguf \
  --local-dir ~/models
```

### Step 3: Launch llama-server with TurboQuant

```bash
# turbo4: 4.7x KV compression, +0.6% perplexity, near-native decode speed
# This is the recommended default for most workloads.
~/turboquant_plus/build/bin/llama-server \
  -m ~/models/qwen3-coder-30b-a3b-q4_k_m.gguf \
  --cache-type-k q8_0 \
  --cache-type-v turbo4 \
  --jinja \
  -ngl 99 \
  -c 262144 \
  -fa on \
  --host 0.0.0.0 \
  --port 8080
```

**Flag reference:**

| Flag | Purpose |
|------|---------|
| `--cache-type-k q8_0` | Key cache quantization (8-bit) |
| `--cache-type-v turbo4` | Value cache TurboQuant level 4 |
| `--jinja` | Enable Jinja2 chat templates (required for tool calling) |
| `-ngl 99` | Offload all layers to Metal GPU |
| `-c 262144` | Context window: 262K tokens |
| `-fa on` | Flash attention (required for large contexts) |

### Step 4: Configure Anvil for TurboQuant

```bash
cd your-project
anvil init    # if not already done
```

Edit `.anvil/config.toml`:

```toml
[provider]
backend = "llama-server"
base_url = "http://localhost:8080/v1"
model = "qwen3-coder-tq4"
```

The model name `qwen3-coder-tq4` matches the bundled TQ4 profile,
which sets `recommended_context = 262144` and the correct sampling
parameters automatically.

### Step 5: Verify

```bash
anvil
# You should see:
#   profile: Qwen3-Coder TQ4 loaded
#     KV cache: K=q8_0 V=turbo4 | context: 262144 tokens
```

Type `/model` to confirm the profile is active and context is 262K.

### Using the Zellij layout (optional)

Anvil ships a bundled Zellij layout that runs llama-server and Anvil
side by side in split panes:

```bash
brew install zellij    # if not installed

# Edit the layout to point to your model and binary:
$EDITOR .anvil/layouts/anvil-tq.kdl

# Launch both llama-server and Anvil in one command:
anvil --zellij anvil-tq
```

The layout manages both processes — closing the Zellij session kills
everything cleanly.

### turbo3 vs turbo4

| Level | KV Compression | Perplexity Impact | Decode Speed | Context (64GB, 30B Q4) |
|-------|---------------|-------------------|--------------|----------------------|
| turbo4 | 4.7x | +0.6% | Near native | 262K |
| turbo3 | 6.0x | +1.2% | -37.9% on pre-M5 | 512K |

**Use turbo4** unless you specifically need maximum context and accept
the decode speed tradeoff. turbo3's speed penalty is significant on
M4 and earlier chips.

To use turbo3, set `model = "qwen3-coder-tq3"` in config.toml and
launch llama-server with `--cache-type-v turbo3 -c 524288`.

---

## MLX Setup

MLX is Apple's machine learning framework optimized for unified memory
on Apple Silicon. It provides native inference without llama.cpp.

### Step 1: Install mlx-lm

```bash
pip install mlx-lm
```

### Step 2: Start the MLX server

```bash
# Qwen3-Coder 30B (4-bit quantized, ~19GB)
mlx_lm.server \
  --model mlx-community/Qwen3-Coder-30B-A3B-4bit \
  --port 8080

# Or a smaller model for faster iteration:
mlx_lm.server \
  --model mlx-community/Qwen2.5-Coder-7B-Instruct-4bit \
  --port 8080
```

The server exposes an OpenAI-compatible API at `http://localhost:8080/v1`.

### Step 3: Configure Anvil for MLX

Edit `.anvil/config.toml`:

```toml
[provider]
backend = "mlx"
base_url = "http://localhost:8080/v1"
model = "mlx-community/Qwen3-Coder-30B-A3B-4bit"
```

The model name containing `mlx-community` matches the bundled MLX
Default profile automatically.

### Step 4: Verify

```bash
anvil
# You should see:
#   profile: MLX Default loaded
```

### MLX caveats

- **tool_choice**: Some MLX models reject the `tool_choice` parameter
  with HTTP 400/422. Anvil automatically retries without it — no
  action needed.
- **Tool calling**: Varies by model. Qwen3-Coder works well. Smaller
  models may produce malformed tool calls.
- **No TurboQuant**: MLX does not support TurboQuant KV cache
  compression. For maximum context, use llama-server with TurboQuant.

### When to use MLX vs llama-server

| | MLX | llama-server + TurboQuant |
|---|-----|--------------------------|
| Setup | `pip install mlx-lm` | Build from source |
| Context | Model default (32-128K) | Up to 512K with turbo3 |
| Speed | Fast (native Metal) | Fast (Metal via GGML) |
| Tool calling | Model-dependent | Reliable with `--jinja` |
| Best for | Quick iteration, smaller models | Maximum context, production |

---

## Daemon Mode

The daemon runs Anvil as a background server. Send prompts from any
terminal without restarting the agent or losing session state.

### Starting the daemon

```bash
# Foreground (see output, Ctrl+C to stop)
anvil daemon start

# Background with nohup
nohup anvil daemon start > /tmp/anvil-daemon.log 2>&1 &

# Or use a systemd service / launchd plist for production
```

The daemon binds a Unix domain socket at:
- Linux: `$XDG_RUNTIME_DIR/anvil/daemon.sock`
- macOS: `/tmp/anvil-$UID/daemon.sock`

The socket is created with `0600` permissions (owner-only access).

### Sending prompts

```bash
# Send a prompt — content streams to stdout, diagnostics to stderr
anvil send "explain the main function in src/main.rs"

# Auto-approve all tool calls (file writes, shell commands)
anvil send -y "fix the failing test in tests/auth.rs"

# Pipe-friendly: stdout is clean content, stderr is diagnostics
anvil send "list all TODO comments" > todos.txt
anvil send "summarize this codebase" | pbcopy
```

### Checking status

```bash
anvil daemon status
# Output:
#   anvil daemon is running
#     pid:     12345
#     session: a1b2c3d4
#     model:   qwen3-coder:30b
#     mode:    coding
#     uptime:  1h 23m 45s
#     socket:  /tmp/anvil-501/daemon.sock
```

### Stopping the daemon

```bash
anvil daemon stop
```

### Session continuity

The daemon persists session state (messages, mode, persona, skills)
to SQLite after every turn. If the daemon restarts, use
`anvil --continue` in interactive mode to resume the same session.

### Daemon with TurboQuant

A typical production setup on a MacBook Pro M4 Max:

```bash
# Terminal 1: Start llama-server with TurboQuant
~/turboquant_plus/build/bin/llama-server \
  -m ~/models/qwen3-coder-30b-a3b-q4_k_m.gguf \
  --cache-type-k q8_0 --cache-type-v turbo4 \
  --jinja -ngl 99 -c 262144 -fa on \
  --host 0.0.0.0 --port 8080

# Terminal 2: Start the Anvil daemon
anvil daemon start

# Terminal 3 (or any terminal): Send prompts
anvil send "refactor the auth module to use JWT"
anvil send -y "add tests for the new JWT auth"
```

---

## Watch Mode

Watch mode monitors your workspace for file changes and triggers
agent turns automatically. No external tools (watchexec, entr) needed.

### Basic usage

```bash
# Watch the current directory
anvil watch

# Custom debounce interval (default: 2 seconds)
anvil watch --debounce 5

# Ignore additional patterns
anvil watch --ignore vendor/ --ignore dist/
```

### What gets filtered automatically

| Category | Filtered |
|----------|----------|
| Version control | `.git/` |
| Build artifacts | `target/`, `node_modules/`, `__pycache__/` |
| Editor artifacts | `.swp`, `.swo`, `.tmp`, `.bak` |
| Hidden files | Dotfiles (except `.env`) |
| Compiled output | `.pyc`, `.pyo`, `.o`, `.a` |

### Debounce

Editors save files in multiple steps (write temp, rename, chmod).
Anvil collects events into a batch and waits for a quiet period
before triggering. The default 2-second debounce prevents spamming
the LLM with intermediate saves.

### Watch mode permissions

Watch mode auto-approves read-only tools (file_read, grep, ls, find,
git_status, git_diff, git_log) but denies mutating tools (file_write,
file_edit, shell, git_commit). The user isn't at the keyboard, so
destructive operations require explicit approval via interactive mode
or `anvil send -y`.

---

## The .anvil/ Directory

Created by `anvil init`. Structure:

```
.anvil/
+-- config.toml          # Provider, agent, and tool settings
+-- context.md           # Injected into system prompt (project notes)
+-- inventory.toml       # Host/service registry (optional)
+-- achievements.json    # Unlocked badges
+-- models/              # Per-model sampling profiles (TOML)
+-- skills/              # Prompt templates (22 bundled)
+-- layouts/             # Zellij terminal layouts (3 bundled)
+-- memory/              # Persistent learned patterns
```

### context.md

Free-form markdown injected into the system prompt. Use it for:
- Project-specific conventions
- Known gotchas
- Architecture notes
- Deployment procedures

The LLM reads this every turn, so keep it concise.

### Launch profiles

Bundle persona + mode + skills + model into a single flag:

```toml
# In config.toml
[[profiles]]
name = "tq"
model = "qwen3-coder-tq4"
mode = "coding"
skills = ["containers", "deploy"]

[[profiles]]
name = "sparkle"
persona = "sparkle"
mode = "creative"
skills = ["kids-first"]
```

```bash
anvil -p tq          # TurboQuant coding setup
anvil -p sparkle     # Kids mode
```

---

## Backends

Anvil connects to any OpenAI-compatible API.

| Backend | URL | Discovery | Best for |
|---------|-----|-----------|----------|
| Ollama | localhost:11434/v1 | /api/tags | Easy setup |
| llama-server | localhost:8080/v1 | /v1/models | TurboQuant, template fidelity |
| MLX | localhost:8080/v1 | /v1/models | Apple Silicon native |
| Custom | any | /v1/models | Remote APIs, vLLM, etc. |

### Switching backends

In `config.toml`:

```toml
[provider]
backend = "llama-server"
base_url = "http://localhost:8080/v1"
model = "qwen3-coder-tq4"
```

Or at runtime:

```
/backend llama http://localhost:8080/v1
```

---

## Model Profiles

Profiles in `.anvil/models/*.toml` set per-model sampling parameters.
Auto-applied when the model name matches `match_patterns`.

### Bundled profiles

| Profile | Match Pattern | Context | Backend |
|---------|--------------|---------|---------|
| Qwen3-Coder | qwen3-coder | 32K | Ollama |
| Qwen3-Coder TQ4 | qwen3-coder-tq4 | 262K | llama-server |
| Qwen3-Coder TQ3 | qwen3-coder-tq3 | 512K | llama-server |
| Qwen3 | qwen3 | 32K | Ollama |
| Devstral | devstral | 32K | Ollama |
| DeepSeek-R1 | deepseek-r1 | 32K | Ollama |
| GLM-4.7-Flash | glm-4 | 16K | llama-server |
| MLX Default | mlx-community | 32K | MLX |

### Profile format

```toml
name = "Qwen3-Coder TQ4"
match_patterns = ["qwen3-coder-tq4", "qwen3-coder-turbo4"]

[sampling]
temperature = 0.7
top_p = 0.95

[context]
max_window = 262144
default_window = 32768

[backend]
preferred = "llama-server"
flags = ["--jinja", "--cache-type-k", "q8_0", "--cache-type-v", "turbo4"]

[kv_cache]
type_k = "q8_0"
type_v = "turbo4"
recommended_context = 262144
```

When `[kv_cache]` is present, `recommended_context` overrides
`context.default_window`.

### Adding a custom profile

Create a `.toml` file in `.anvil/models/`. The `match_patterns` field
does case-insensitive substring matching against the active model name.

---

## Skills

Skills are markdown files that inject domain knowledge into the
system prompt.

### Using skills

```
/skill              # List all skills (grouped by category)
/skill containers   # Activate the containers skill
/skill clear        # Deactivate all skills
/skill verify containers  # Run the skill's verification command
```

### Bundled skills (22)

| Category | Skills |
|----------|--------|
| Infrastructure | containers, server-admin, sops-age, deploy-fish, tailscale, caddy-cloudflare, restic-backup, grafana, prometheus, deploy |
| Dev Tools | nvim, zellij, fish, git-workflow |
| Meta | verify-all, verify-shell, verify-files, learn-anvil, learn-rust |
| Kids | kids-first, kids-story, kids-game |

### Writing a skill

Create a `.md` file in `.anvil/skills/`:

```markdown
---
description: "Short description for /skill listing"
category: infrastructure
tags: [docker, compose]
env:
  - DOCKER_HOST
verify: "command -v docker"
---
# Skill Name

Instructions for the LLM go here.
```

**Frontmatter fields** (all optional):
- `description` — shown in `/skill` listing
- `category` — groups skills in the listing
- `tags` — for future search/filtering
- `env` — environment variables passed to shell when skill is active
- `verify` — command to check prerequisites

### Environment passthrough

When a skill declares `env: [DOCKER_HOST]`, that variable is passed
through to shell commands while the skill is active. Deactivating the
skill removes the passthrough. Base safe vars (PATH, HOME, USER, LANG,
TERM) always pass through.

---

## Autonomous Mode

Run a task repeatedly until a verification command passes:

```bash
# Fix tests — retry until cargo test passes
anvil run -p "fix all failing tests" -a --verify "cargo test"

# Deploy — retry until compose is healthy
anvil run -p "deploy the stack" -a --verify "docker compose ps" --max-iterations 5

# With time limit
anvil run -p "optimize the build" -a --verify "cargo build" --max-minutes 15
```

### How it works

1. Send prompt to LLM with auto-approved tool calls
2. LLM executes tools (reads files, runs commands, edits code)
3. Run the `--verify` command
4. If verify passes (exit 0) → done
5. If verify fails → feed failure output back to LLM → goto 1
6. Stop if limits hit (iterations, tokens, time)

### Guardrails

- `--max-iterations N` (default: 10) — hard limit on retry count
- `--max-minutes N` (default: 30) — wall-clock time limit
- Token budget: 100,000 tokens per autonomous run
- LLM can declare `[ANVIL:DONE]` to trigger final verification

---

## Interactive Commands

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/stats` | Token usage, model, backend info |
| `/model [name]` | Show or switch model |
| `/backend [type url]` | Show or switch backend |
| `/backend start llama <model>` | Start a managed llama-server |
| `/backend stop` | Stop the managed backend |
| `/skill [name]` | List, activate, or verify skills |
| `/ralph <prompt> --verify <cmd>` | Autonomous mode (Ralph Loop) |
| `/clear` | Compact conversation context via LLM summary |
| `/think` | Toggle thinking block visibility |
| `/route [tool model]` | Show or set model routing |
| `/memory` | List stored patterns (categorized) |
| `/memory add <pattern>` | Save a new pattern |
| `/memory search <keyword>` | Search memories by keyword |
| `/mcp` | List MCP servers and tools |
| `/persona [name]` | Activate persona (sparkle, bolt, codebeard, homelab) |
| `/persona clear` | Deactivate persona |
| `/mode [coding\|creative]` | Switch operating mode |
| `/selftest` | Run self-diagnostics |
| `/inventory` | Show host/service inventory |
| `/history` | List recent sessions |
| `/end` | End session and exit |

---

## Configuration Reference

### config.toml

```toml
[provider]
backend = "ollama"                     # ollama | llama-server | mlx | custom
base_url = "http://localhost:11434/v1"
model = "qwen3-coder:30b"
# api_key = "$OPENAI_API_KEY"          # optional, $VAR syntax for env vars

[agent]
max_tokens = 200000                    # session token budget
context_window = 8192                  # overridden by model profile
loop_detection_limit = 10              # max identical consecutive tool calls
auto_compact_threshold = 80            # auto-compact at 80% (0 = disabled)
# kids_workspace = "~/kids-projects"   # sandbox for kids personas

[tools]
shell_timeout_secs = 30               # per-command timeout
output_limit = 10000                  # max bytes before truncation

# Launch profiles — one-command setup
# [[profiles]]
# name = "tq"
# model = "qwen3-coder-tq4"
# mode = "coding"
# skills = ["containers"]

# MCP (Model Context Protocol) — connect external tool servers
# [[mcp.servers]]
# name = "filesystem"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
```

---

## Architecture

```
anvil (binary)
  CLI, interactive mode, daemon, watch, IPC client
    |
anvil-agent
  Agent loop, skills, personas, achievements, sessions,
  Event enum, dispatch loop
    |
  +-- anvil-llm      HTTP client, SSE streaming, retry, tool_choice
  +-- anvil-tools    11 tools, executor, permissions, hooks
  +-- anvil-mcp      MCP client, JSON-RPC over stdio
  +-- anvil-config   Settings, profiles, bundled skills/layouts
```

Dependencies flow downward. `anvil-config` and `anvil-mcp` have no
internal dependencies.

### Daemon architecture

```
anvil daemon start
  |
  +-- UDS Accept Loop (tokio task)
  |     accepts connections, reads Request, enqueues DaemonTask
  |
  +-- Signal Handler (tokio task)
  |     SIGINT/SIGTERM -> DaemonTask::Shutdown
  |
  +-- Dispatch Loop (main task, owns &mut Agent)
        processes DaemonTask sequentially
        calls dispatch_event() -> agent.turn()
        streams AgentEvent back via reply channel

anvil send "prompt"
  |
  +-- connects to UDS
  +-- sends Request::Prompt
  +-- streams Response frames to stdout/stderr
```

The dispatch loop is the sole owner of the Agent. No concurrent
access, no Arc/Mutex. Tasks queue naturally in the mpsc channel.

### How to add a new tool

1. Define the tool schema in `crates/anvil-tools/src/definitions.rs`
2. Implement the tool function in `crates/anvil-tools/src/tools.rs`
3. Add the dispatch case in `crates/anvil-tools/src/executor.rs`
4. Classify as read-only or mutating in `crates/anvil-tools/src/permission.rs`
5. Add tests in `crates/anvil-tools/tests/tool_tests.rs`
