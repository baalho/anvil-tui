# Anvil Manual

A local-first coding agent for local models. Runs offline, connects to
Ollama, llama-server, or MLX.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    anvil (binary)                        │
│  CLI parsing, interactive mode, autonomous loop          │
├─────────────────────────────────────────────────────────┤
│                   anvil-agent                            │
│  Agent loop, skills, system prompt, sessions, autonomy   │
├──────────────────────┬──────────────────────────────────┤
│     anvil-tools      │          anvil-llm                │
│  7 tools, executor,  │  HTTP client, SSE streaming,      │
│  permissions, trunc  │  retry, token usage               │
├──────────────────────┴──────────────────────────────────┤
│                   anvil-config                           │
│  Settings, provider config, model profiles, bundled      │
│  skills, harness directory management                    │
└─────────────────────────────────────────────────────────┘
```

Dependencies flow downward. `anvil-config` has no internal dependencies.
`anvil-llm` depends on `anvil-config`. `anvil-tools` depends on nothing
internal. `anvil-agent` depends on all three. The `anvil` binary depends
on `anvil-agent`.

## macOS Setup (M-series Mac)

### Prerequisites

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install Ollama (easiest backend)
brew install ollama
ollama serve &                    # start in background

# Pull a model (pick one)
ollama pull qwen3-coder:30b      # 19GB — best coding agent (fits 64GB)
ollama pull devstral              # 14GB — #1 SWE-Bench open-source
ollama pull deepseek-r1:32b      # 20GB — reasoning model
ollama pull qwen3:8b             # 5GB  — lightweight general purpose
```

### Optional: llama-server (for GLM-4.7-Flash or better template fidelity)

```bash
# Install llama.cpp via Homebrew
brew install llama.cpp

# Download a GGUF model (e.g. from Hugging Face)
# Then start the server:
llama-server \
  --model ~/models/GLM-4.7-Flash-Q4_K_M.gguf \
  --port 8080 \
  --jinja \
  --ctx-size 16384 \
  --n-gpu-layers 99              # use all Metal GPU layers
```

### Optional: MLX (Apple Silicon native)

```bash
# Install mlx-lm
pip install mlx-lm

# Start the server
mlx_lm.server --model mlx-community/Qwen3-Coder-30B-4bit --port 8080
```

### Build and install Anvil

```bash
# Clone and build
git clone https://github.com/baalho/anvil-cli.git
cd anvil-cli
cargo build --release

# Install to PATH
cp target/release/anvil ~/.local/bin/
# or: cp target/release/anvil /usr/local/bin/

# Verify
anvil --version
```

### Initialize a project

```bash
cd your-project
anvil init                        # creates .anvil/ with config, skills, profiles

# Start interactive mode
anvil

# Run a single prompt
anvil run -p "explain this codebase"

# Run with auto-approve
anvil run -p "fix the build" -y
```

### Ollama context window

Ollama defaults to 2048 tokens context — far too small for coding tasks.
Set a larger context window:

```bash
# Option 1: environment variable (applies to all models)
export OLLAMA_NUM_CTX=16384

# Option 2: per-model via Modelfile
echo 'FROM qwen3-coder:30b
PARAMETER num_ctx 32768' | ollama create qwen3-coder-32k -f -
```

Anvil's model profiles set `default_window` but Ollama must also be configured
to actually allocate that context.

### Recommended dev tools (optional)

```bash
brew install fish neovim zellij   # shell, editor, multiplexer
# Anvil ships skills for all three — activate with /skill nvim etc.
```

## The .anvil/ Directory

Created by `anvil init`. Structure:

```
.anvil/
├── config.toml          # Provider, agent, and tool settings
├── context.md           # Injected into system prompt (project info, lessons learned)
├── models/              # Per-model sampling profiles
│   ├── qwen3-coder.toml
│   ├── qwen3.toml
│   ├── devstral.toml
│   ├── deepseek-r1.toml
│   └── glm-4.7-flash.toml
├── skills/              # Prompt template skills (14 bundled)
│   ├── docker.md
│   ├── server-admin.md
│   ├── nvim.md
│   ├── verify-all.md
│   └── ...
└── memory/              # Reserved for future use
```

## Backends

Anvil connects to any OpenAI-compatible API. Three backends are supported:

| Backend | URL | Discovery | Best for |
|---------|-----|-----------|----------|
| Ollama | localhost:11434/v1 | /api/tags | Easy setup, auto-pull |
| llama-server | localhost:8080/v1 | /v1/models | Template fidelity (GLM-4.7) |
| MLX | localhost:8080/v1 | /v1/models | Apple Silicon performance |

### Switching backends

In `config.toml`:
```toml
[provider]
backend = "llama-server"
base_url = "http://localhost:8080/v1"
model = "glm-4.7-flash"
```

Or at runtime:
```
/backend llama http://localhost:8080/v1
```

### Starting llama-server (example with GLM-4.7-Flash)

```bash
llama-server \
  --model ~/models/GLM-4.7-Flash-Q4_K_M.gguf \
  --port 8080 \
  --jinja \
  --ctx-size 16384 \
  --n-gpu-layers 99              # use all Metal GPU layers on Apple Silicon
```

## Model Profiles

Profiles in `.anvil/models/*.toml` set per-model sampling parameters.
When Anvil detects a model name matching a profile's `match_patterns`,
it automatically applies the profile's settings.

Bundled profiles (all fit on 64GB M4 Max):

| Profile | Params | VRAM (Q4) | Context | Best for |
|---------|--------|-----------|---------|----------|
| Qwen3-Coder 30B | 30B MoE | 19GB | 256K | Coding agent tasks |
| Qwen3 | 0.6B-235B | varies | 256K | General purpose |
| Devstral 24B | 24B | 14GB | 128K | SWE-Bench tasks |
| DeepSeek-R1 32B | 32B | 20GB | 131K | Reasoning, chain-of-thought |
| GLM-4.7-Flash | 30B MoE | 18GB | 200K | Tool calling |

```toml
# .anvil/models/qwen3-coder.toml
name = "Qwen3-Coder"
match_patterns = ["qwen3-coder", "Qwen3-Coder"]

[sampling]
temperature = 0.7
top_p = 0.95

[context]
max_window = 262144
default_window = 32768

[backend]
preferred = "ollama"
```

### Adding a new profile

Create a `.toml` file in `.anvil/models/`. The `match_patterns` field
does case-insensitive substring matching against the active model name.

## Skills

Skills are markdown files that inject domain knowledge into the system prompt.

### Using skills

```
/skill              # List all skills (grouped by category)
/skill docker       # Activate the Docker skill
/skill clear        # Deactivate all skills
/skill verify docker  # Run Docker's verification command
```

### Writing a skill

Create a `.md` file in `.anvil/skills/`:

```markdown
---
description: "Short description for /skill listing"
category: infrastructure
tags: [docker, containers]
env:
  - DOCKER_HOST
verify: "docker info"
---
# Skill Name

Instructions for the LLM go here.
```

**Frontmatter fields** (all optional):
- `description` — shown in `/skill` listing
- `category` — groups skills in the listing
- `tags` — for future search/filtering
- `env` — environment variables passed to shell when skill is active
- `verify` — command to check prerequisites (`/skill verify <name>`)

### Environment passthrough

When a skill declares `env: [DOCKER_HOST]`, that variable is passed through
to shell commands while the skill is active. Deactivating the skill removes
the passthrough. Base safe vars (PATH, HOME, USER, LANG, TERM) always pass.

## Autonomous Mode (Ralph Loop)

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

## Interactive Commands

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/stats` | Token usage, model, backend, env passthrough |
| `/model [name]` | Show or switch model |
| `/backend [type url]` | Show or switch backend |
| `/skill [name]` | List, activate, or verify skills |
| `/ralph` | Show autonomous mode usage |
| `/history` | List recent sessions |
| `/clear` | Compact context (placeholder) |
| `/end` | End session and exit |

## How to Add a New Tool

1. Define the tool schema in `crates/anvil-tools/src/definitions.rs`
2. Implement the tool function in `crates/anvil-tools/src/tools.rs`
3. Add the dispatch case in `crates/anvil-tools/src/executor.rs`
4. Add tests in `crates/anvil-tools/tests/tool_tests.rs`
5. Update the tool count in system prompt (`crates/anvil-agent/src/system_prompt.rs`)

## How the Agent Loop Works

```
User input
    │
    ▼
┌─────────────────┐
│ Build ChatRequest│ ← system prompt + messages + tool definitions
│ with sampling    │ ← from active model profile
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ LLM API call    │ ← SSE streaming with retry
│ (chat_stream)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐     ┌──────────────┐
│ Response has     │─yes─▶ Execute tool  │
│ tool_calls?      │     │ (with perms)  │
└────────┬────────┘     └──────┬───────┘
         │no                    │
         ▼                      │ tool result
┌─────────────────┐     ┌──────▼───────┐
│ Display content  │     │ Add to msgs  │
│ Turn complete    │     │ Loop back ↑  │
└─────────────────┘     └──────────────┘
```

## Configuration Reference

### config.toml

```toml
[provider]
backend = "ollama"                    # ollama | llama-server | mlx | custom
base_url = "http://localhost:11434/v1"
model = "qwen3-coder:30b"
# api_key = "$OPENAI_API_KEY"        # optional, $VAR syntax for env vars

[agent]
max_tokens = 200000                   # session token budget
warn_threshold_pct = 80              # warn at this % of context window
loop_detection_limit = 10            # max identical consecutive tool calls
context_window = 8192                # overridden by model profile

[tools]
shell_timeout_secs = 30              # per-command timeout
file_timeout_secs = 5                # file operation timeout
output_limit = 10000                 # max bytes before truncation
```
