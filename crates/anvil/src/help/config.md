# Configuration

Anvil is configured via `.anvil/config.toml` in your project directory.
Run `anvil init` to create the default configuration.

## Provider Settings

```toml
[provider]
backend = "ollama"           # ollama, llama-server, mlx, custom
base_url = "http://localhost:11434/v1"
model = "qwen3-coder:30b"
# api_key = "sk-..."         # for remote APIs
# api_key_env = "OPENAI_API_KEY"  # read from env var
```

## Agent Settings

```toml
[agent]
loop_limit = 3               # max repeated identical tool calls
auto_compact_threshold = 90  # auto-compact at N% context usage
```

## Tool Settings

```toml
[tool]
shell_timeout_secs = 30      # default shell command timeout
output_limit = 50000         # max tool output bytes
```

## Model Profiles

Per-model sampling parameters in `.anvil/models/*.toml`:

```toml
name = "qwen3-coder"
match_patterns = ["qwen3-coder", "qwen3"]

[sampling]
temperature = 0.7
top_p = 0.95

[context]
default_window = 32768
```

## Environment Variables

- OLLAMA_NUM_CTX — Ollama context window size (default: 2048, recommend: 32768)
- ANVIL_LOG — log level (debug, info, warn, error)
