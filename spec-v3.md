# Anvil v3 — Multi-Backend, Skills Pack, Autonomous Mode & Documentation

## Vision

Anvil evolves from an Ollama-only coding agent into a **local-first Command & Control
plane** that works with any OpenAI-compatible backend (Ollama, llama-server, mlx_lm.server),
ships a curated skill pack for infrastructure work (Docker, monitoring, dev tooling),
supports autonomous task completion (Ralph Loop), and serves as its own training manual.

The MacBook Pro M4 Max (64GB) is the primary target. Everything works offline/airgapped.

---

## Problem Statement

Anvil v2 is a functional coding agent, but has these gaps preventing daily-driver use
as a swiss-army-knife C2 plane:

1. **Ollama-only** — hardcoded to `localhost:11434/v1`, uses Ollama-specific `/api/tags`.
   Unsloth explicitly warns against GLM-4.7-Flash on Ollama due to chat template bugs.
   llama-server and MLX backends need different URLs, ports, and sampling parameters.

2. **No model profiles** — every model needs different temp, top_p, min_p, repeat_penalty,
   context window. Currently one global config. Switching models means editing settings.toml.

3. **No infrastructure skills** — only verification skills exist. No Docker, Grafana,
   server admin, or dev tooling skills. The agent can't help with infrastructure tasks.

4. **No autonomous mode** — single-turn interaction only. Can't say "fix all tests" and
   walk away. No Ralph Loop (retry-until-verification-passes).

5. **Shell env is locked down** — `env_clear()` blocks DOCKER_HOST, KUBECONFIG,
   SSH_AUTH_SOCK. Infrastructure skills can't function.

6. **No documentation** — code has minimal comments. No training manual. A mechanical
   engineer learning Rust can't understand the codebase without guidance.

7. **Skills lack structure** — no frontmatter, no metadata, no env declarations,
   no verification commands. Just raw markdown.

---

## Architecture Decisions

### AD-1: Backend Strategy — "Connect Only" (Phase 1)

Anvil connects to whatever OpenAI-compatible server is running. User starts
llama-server, Ollama, or mlx_lm.server manually. Anvil just needs a URL.

**Rationale**: All three backends expose the same `/v1/chat/completions` endpoint.
Anvil's `LlmClient` already speaks this protocol. The real value is model profiles
(correct sampling params per model) and easy switching, not process management.

**Future**: Phase 2 adds lifecycle management (`/backend start llama glm-4.7`).

### AD-2: Model Profiles in `.anvil/models/`

TOML files per model with sampling params, backend hints, and context window.
Loaded by name via `/model` command. Bundled defaults created by `anvil init`.

### AD-3: Structured Skill Frontmatter

Skills gain YAML frontmatter for metadata: description, category, required env vars,
verification command, and tags. Backward-compatible — skills without frontmatter
still work exactly as before.

### AD-4: Ralph Loop as `--autonomous` Flag

Built into the agent loop. Agent keeps running turns until a verification command
passes (defined by skill or CLI flag), or hits hard limits (iterations, tokens, time).
The LLM can also declare DONE explicitly.

### AD-5: Per-Skill Environment Passthrough

Skills declare required env vars in frontmatter. When a skill is active, those vars
are passed through to the shell tool. Base safe vars always pass through.

### AD-6: Documentation as Code + Skills

Every public Rust item gets doc comments. Each skill file doubles as documentation
(explains concepts, shows examples, then provides the prompt template). A MANUAL.md
ties everything together as a walkthrough.

---

## Requirements

### R1: Multi-Backend Support

**R1.1** — `ProviderConfig` gains a `backend` field: `ollama | llama-server | mlx | custom`.
Default: `ollama`.

**R1.2** — Auto-detection adapts per backend:
- `ollama`: query `/api/tags` (existing behavior)
- `llama-server`: query `/v1/models` (OpenAI standard)
- `mlx`: query `/v1/models`
- `custom`: skip auto-detection

**R1.3** — `/backend` slash command shows current backend info (URL, type, status).
Accepts optional argument to switch: `/backend llama http://localhost:8080/v1`.

**R1.4** — `ChatRequest` gains optional fields for sampling params: `top_p`, `min_p`,
`repeat_penalty`, `top_k`. These are populated from model profiles when available.

**R1.5** — Settings.toml `[provider]` section accepts `backend = "llama-server"` etc.

### R2: Model Profiles

**R2.1** — `.anvil/models/` directory contains TOML profile files. Format:

```toml
# .anvil/models/glm-4.7-flash.toml
name = "GLM-4.7-Flash"
match_patterns = ["glm-4.7-flash", "GLM-4.7-Flash"]

[sampling]
temperature = 0.7
top_p = 1.0
min_p = 0.01
repeat_penalty = 1.0

[context]
window = 202752
default_window = 16384  # practical default for most tasks

[backend]
preferred = "llama-server"
flags = ["--jinja"]
notes = "Unsloth warns against Ollama for this model due to chat template issues"
```

**R2.2** — `anvil init` creates bundled profiles for: `qwen3-coder`, `qwen3`,
`devstral`, `deepseek-r1`, `glm-4.7-flash`. User can add more.

**R2.3** — When a model is selected (via `/model` or auto-detect), Anvil matches against
`match_patterns` and loads the profile's sampling params automatically.

**R2.4** — `/model` command shows loaded profile info when a profile matches.

**R2.5** — Profile loading is optional. Unknown models use global defaults from settings.toml.

### R3: Structured Skill Frontmatter

**R3.1** — Skills support optional YAML frontmatter between `---` delimiters:

```markdown
---
description: "Manage Docker containers, images, and compose stacks"
category: infrastructure
tags: [docker, containers, devops]
env:
  - DOCKER_HOST
  - DOCKER_CONFIG
verify: "docker info --format '{{.ServerVersion}}'"
---
# Docker Management

...prompt content...
```

**R3.2** — `Skill` struct gains fields: `category: Option<String>`,
`tags: Vec<String>`, `required_env: Vec<String>`, `verify_command: Option<String>`.

**R3.3** — `parse_skill_file()` extracts frontmatter if present, falls back to
current heading-based parsing if not. 100% backward compatible.

**R3.4** — `/skill` listing groups by category when categories exist.

**R3.5** — `/skill verify <name>` runs the skill's verify command and reports pass/fail.

### R4: Curated Skill Pack

**R4.1** — `anvil init` creates the following skills in `.anvil/skills/`:

**Infrastructure (category: infrastructure)**
- `docker.md` — Container lifecycle, compose, images, volumes, networks, logs
- `docker-compose.md` — Multi-service orchestration, env files, build contexts
- `server-admin.md` — systemctl, journalctl, disk/memory/CPU monitoring, SSH
- `grafana.md` — Dashboard provisioning, datasource config, alerting rules
- `prometheus.md` — Scrape configs, recording rules, alertmanager

**Dev Tooling (category: dev-tools)**
- `nvim.md` — Config editing, plugin management, LSP setup, keybindings
- `zellij.md` — Layout management, pane/tab control, session handling
- `fish.md` — Shell config, abbreviations, functions, completions
- `git-workflow.md` — Branch strategy, interactive rebase, bisect, worktrees

**Anvil Meta (category: meta)**
- `verify-all.md` — (existing, updated with frontmatter)
- `verify-shell.md` — (existing, updated with frontmatter)
- `verify-files.md` — (existing, updated with frontmatter)
- `learn-anvil.md` — Guided tutorial: how Anvil works, how to write skills
- `learn-rust.md` — Rust concepts explained through Anvil's codebase

**R4.2** — Each skill file follows the dual-purpose format:
1. Frontmatter with metadata
2. Concept explanation section (teaches the user)
3. Prompt template section (instructs the LLM)
4. Examples section (shows real usage)

**R4.3** — Skills are self-contained. No external dependencies or web lookups required.

### R5: Autonomous Mode (Ralph Loop)

**R5.1** — New CLI flag: `--autonomous` (short: `-a`). Requires a prompt.

```bash
anvil run -p "fix all failing tests" --autonomous --verify "cargo test"
anvil run -p "deploy the monitoring stack" -a --verify "docker compose ps"
```

**R5.2** — Autonomous mode runs the agent loop repeatedly:
1. Execute a turn (send prompt + context to LLM, execute tool calls)
2. After turn completes, run the `--verify` command
3. If verify exits 0 → success, stop, report results
4. If verify exits non-zero → feed the failure output back as a new user message,
   run another turn
5. Repeat until verify passes or limits hit

**R5.3** — Hard limits (all configurable via CLI flags):
- `--max-iterations N` (default: 10)
- `--max-tokens N` (default: 100000)
- `--max-minutes N` (default: 30)

**R5.4** — The LLM can also declare completion by outputting `[ANVIL:DONE]` in its
response. This triggers the verify command one final time.

**R5.5** — `AgentEvent::AutonomousIteration { iteration, max, verify_passed }` event
for UI feedback.

**R5.6** — `AgentEvent::AutonomousComplete { iterations, passed, summary }` event
when the loop finishes.

**R5.7** — Interactive mode also supports autonomous via `/ralph` slash command:
`/ralph fix the build --verify "cargo build"` — runs Ralph Loop inline.

**R5.8** — Auto-approve (`--yes`) is implied in autonomous mode. The agent can't
wait for human permission mid-loop.

### R6: Environment Passthrough

**R6.1** — When a skill with `env` frontmatter is active, those env vars are added
to the shell tool's passthrough list for the duration of the session.

**R6.2** — Base safe vars (PATH, HOME, USER, LANG, TERM + platform-specific) always
pass through regardless of skills.

**R6.3** — `ToolExecutor` gains a method `set_extra_env(vars: Vec<String>)` that
the agent calls when skills are activated/deactivated.

**R6.4** — `/stats` shows currently passed-through env vars when skills are active.

### R7: Documentation & Training Material

**R7.1** — Every public struct, enum, trait, and function in all 5 crates gets
`///` doc comments explaining purpose, usage, and non-obvious behavior.

**R7.2** — Complex algorithms (retry logic, SSE parsing, tool call accumulation,
loop detection, context estimation) get block comments explaining the "why".

**R7.3** — `MANUAL.md` at project root: a walkthrough covering:
- Architecture overview (crate dependency graph, data flow)
- How to add a new tool
- How to write a skill
- How to add a model profile
- How to use autonomous mode
- How the agent loop works (with ASCII diagram)
- Glossary of terms

**R7.4** — `learn-anvil.md` skill: a guided exercise that uses Anvil to explore
its own codebase. Teaches by doing.

**R7.5** — `learn-rust.md` skill: explains Rust concepts (ownership, traits,
async, error handling) using Anvil's actual code as examples.

**R7.6** — `LESSONS_LEARNED.md`: structured after-action review documenting:
- What worked well in Anvil's development
- What didn't work (ratatui TUI, argv shell commands, etc.)
- Patterns to reuse
- Anti-patterns to avoid
- The Ralph Loop methodology explained

**R7.7** — `.anvil/context.md` gains a "Lessons Learned" section that is injected
into the system prompt. This is the "prompt for yourself" — a self-reminder that
Anvil reads on every session start:
- Shell commands must be strings, not argv arrays
- Always read a file before editing
- Prefer `file_edit` over `file_write` for existing files
- Check exit codes before proceeding
- When stuck in a loop, try a different approach
- Verify changes with the appropriate tool before declaring done
- For GLM-4.7-Flash: use temp=0.7, top_p=1.0, disable repeat penalty
- For autonomous mode: check verification output carefully before next iteration

This section is auto-generated by `anvil init` but user-editable.

### R8: Testing

**R8.1** — Unit tests for all new code:
- Model profile parsing and matching (`anvil-config`)
- Skill frontmatter parsing (`anvil-agent`)
- Env passthrough logic (`anvil-tools`)
- Autonomous loop state machine (`anvil-agent`)
- Backend detection per type (`anvil-llm`)
- Sampling param injection into ChatRequest (`anvil-llm`)

**R8.2** — Integration tests with mock HTTP server:
- Full agent turn with tool calls against mock `/v1/chat/completions`
- Autonomous loop: mock server returns tool calls, verify command passes on 3rd try
- Backend switching: verify correct URL/params per backend type

**R8.3** — Verification skills updated with frontmatter:
- `verify-all.md` gains `verify: "echo ok"` frontmatter
- New `verify-docker.md` skill with `verify: "docker info"`
- New `verify-backend.md` skill that checks current backend connectivity

**R8.4** — All existing 66 tests continue to pass unchanged.

---

## Acceptance Criteria

1. `anvil init` creates `.anvil/models/` with 4 bundled profiles and `.anvil/skills/`
   with 14+ skill files (infrastructure + dev tools + meta)
2. `/model glm-4.7-flash` loads the profile and applies sampling params automatically
3. `/backend` shows current backend type and URL
4. `/backend llama http://localhost:8080/v1` switches backend
5. Skills with frontmatter parse correctly; skills without frontmatter still work
6. `/skill` groups skills by category
7. `/skill verify docker` runs `docker info` and reports pass/fail
8. `anvil run -p "fix tests" -a --verify "cargo test" --max-iterations 5` runs
   the Ralph Loop and stops when tests pass or iterations exhausted
9. Docker skill activates DOCKER_HOST env passthrough; deactivating clears it
10. Every public item has doc comments; `cargo doc` generates clean documentation
11. MANUAL.md exists and covers all listed topics
12. All new code has unit tests; total test count increases by 30+
13. All 66 existing tests still pass
14. `cargo clippy --all-targets -- -D warnings` passes with zero warnings
15. `cargo fmt` produces no changes

---

## Implementation Plan

### Phase 1: Foundation (model profiles + backend abstraction)

| # | Task | Crate | Files |
|---|------|-------|-------|
| 1 | Add `BackendKind` enum and sampling fields to `ProviderConfig` | anvil-config | provider.rs |
| 2 | Create `ModelProfile` struct and `load_profiles()` in new module | anvil-config | profiles.rs (new) |
| 3 | Add `top_p`, `min_p`, `repeat_penalty`, `top_k` to `ChatRequest` | anvil-llm | message.rs |
| 4 | Inject sampling params from active profile in `LlmClient` | anvil-llm | client.rs |
| 5 | Adapt `auto_detect_model()` per backend kind | anvil (bin) | main.rs |
| 6 | Create bundled profile TOML content for 4 models | anvil-config | profiles.rs |
| 7 | Update `init_harness()` to create `.anvil/models/` with profiles | anvil-config | lib.rs |
| 8 | Add `/backend` slash command | anvil (bin) | commands.rs |
| 9 | Unit tests for profile parsing, matching, sampling injection | anvil-config, anvil-llm | tests/ |

### Phase 2: Structured Skills + Env Passthrough

| # | Task | Crate | Files |
|---|------|-------|-------|
| 10 | Add YAML frontmatter parsing to `parse_skill_file()` | anvil-agent | skills.rs |
| 11 | Extend `Skill` struct with category, tags, env, verify fields | anvil-agent | skills.rs |
| 12 | Add `set_extra_env()` to `ToolExecutor` | anvil-tools | executor.rs |
| 13 | Wire skill activation → env passthrough in `Agent` | anvil-agent | agent.rs |
| 14 | Update shell tool to merge extra env vars | anvil-tools | tools.rs |
| 15 | Add `/skill verify <name>` command | anvil (bin) | commands.rs |
| 16 | Update `/skill` listing to group by category | anvil (bin) | commands.rs |
| 17 | Unit tests for frontmatter parsing, env passthrough | anvil-agent, anvil-tools | tests/ |

### Phase 3: Curated Skill Pack

| # | Task | Crate | Files |
|---|------|-------|-------|
| 18 | Write docker.md skill | — | .anvil/skills/ content |
| 19 | Write docker-compose.md skill | — | .anvil/skills/ content |
| 20 | Write server-admin.md skill | — | .anvil/skills/ content |
| 21 | Write grafana.md skill | — | .anvil/skills/ content |
| 22 | Write prometheus.md skill | — | .anvil/skills/ content |
| 23 | Write nvim.md skill | — | .anvil/skills/ content |
| 24 | Write zellij.md skill | — | .anvil/skills/ content |
| 25 | Write fish.md skill | — | .anvil/skills/ content |
| 26 | Write git-workflow.md skill | — | .anvil/skills/ content |
| 27 | Update existing verify-*.md with frontmatter | — | .anvil/skills/ content |
| 28 | Bundle all skill content into `init_harness()` | anvil-config | lib.rs |

### Phase 4: Autonomous Mode (Ralph Loop)

| # | Task | Crate | Files |
|---|------|-------|-------|
| 29 | Add `--autonomous`, `--verify`, `--max-iterations`, `--max-minutes` CLI flags | anvil (bin) | main.rs |
| 30 | Add `AutonomousConfig` struct | anvil-agent | agent.rs (or autonomous.rs new) |
| 31 | Implement `run_autonomous()` method on Agent | anvil-agent | autonomous.rs (new) |
| 32 | Add `AgentEvent::AutonomousIteration` and `AgentEvent::AutonomousComplete` | anvil-agent | agent.rs |
| 33 | Detect `[ANVIL:DONE]` marker in LLM output | anvil-agent | autonomous.rs |
| 34 | Add `/ralph` slash command for interactive autonomous mode | anvil (bin) | commands.rs |
| 35 | Wire autonomous mode into `cmd_run()` | anvil (bin) | main.rs |
| 36 | Display autonomous progress in interactive.rs | anvil (bin) | interactive.rs |
| 37 | Unit + integration tests for autonomous loop | anvil-agent | tests/ |

### Phase 5: Documentation & Training

| # | Task | Crate | Files |
|---|------|-------|-------|
| 38 | Add doc comments to all public items in anvil-config | anvil-config | all .rs files |
| 39 | Add doc comments to all public items in anvil-llm | anvil-llm | all .rs files |
| 40 | Add doc comments to all public items in anvil-tools | anvil-tools | all .rs files |
| 41 | Add doc comments to all public items in anvil-agent | anvil-agent | all .rs files |
| 42 | Add doc comments to all public items in anvil (bin) | anvil | all .rs files |
| 43 | Write MANUAL.md | — | MANUAL.md |
| 44 | Write learn-anvil.md skill | — | .anvil/skills/ content |
| 45 | Write learn-rust.md skill | — | .anvil/skills/ content |
| 46 | Write LESSONS_LEARNED.md | — | LESSONS_LEARNED.md |
| 47 | Update README.md with new features | — | README.md |

### Phase 6: Verification & Polish

| # | Task | Crate | Files |
|---|------|-------|-------|
| 48 | Run full test suite, fix any failures | all | — |
| 49 | Run cargo clippy, fix all warnings | all | — |
| 50 | Run cargo fmt | all | — |
| 51 | Run cargo doc, verify clean output | all | — |
| 52 | Verify `anvil init` creates complete harness | — | manual test |
| 53 | Update spec.md (replace v2 with v3) | — | spec.md |

---

## Devil's Advocate: Risks & Mitigations

### Risk 1: Skill content quality
Skills are prompt templates. Their effectiveness depends entirely on the LLM's
ability to follow instructions. A 7B model may ignore complex Docker skills.
**Mitigation**: Skills include explicit step-by-step instructions. Verification
commands provide a feedback loop. Ralph Loop retries on failure.

### Risk 2: Autonomous mode runaway
Ralph Loop with `--yes` auto-approve could execute destructive commands repeatedly.
**Mitigation**: Hard limits on iterations, tokens, and wall-clock time. Verification
command is read-only by convention. Skills declare safe env subsets.

### Risk 3: Frontmatter parsing complexity
YAML frontmatter adds a dependency and parsing edge cases.
**Mitigation**: Use `serde_yaml` (well-maintained). Frontmatter is optional —
skills without it work exactly as before. Strict `---` delimiter matching.

### Risk 4: Model profile staleness
Bundled profiles may become outdated as models evolve.
**Mitigation**: Profiles are user-editable TOML files, not compiled in. Users can
update or add profiles without rebuilding Anvil.

### Risk 5: Env passthrough security
Passing DOCKER_HOST or SSH_AUTH_SOCK to shell commands expands the attack surface.
**Mitigation**: Only passed when a skill explicitly declares them AND the skill is
actively loaded. User must consciously activate the skill. `/stats` shows what's
being passed through.

### Risk 6: Documentation maintenance burden
Comprehensive doc comments can become stale as code evolves.
**Mitigation**: Doc comments focus on "why" not "what". `cargo doc` warnings catch
missing docs. Skills-as-docs are tested via verification commands.

---

## Non-Goals (Explicitly Out of Scope)

- Anthropic/Google/proprietary API support (all backends must be OpenAI-compatible)
- Backend lifecycle management (starting/stopping llama-server) — deferred to v4
- Web UI or Electron app
- MCP client/server protocol
- Fine-tuning or training workflows
- Multi-agent orchestration
- Sandboxed execution (Docker-in-Docker)

---

## Dependency Changes

```
cargo add serde_yaml          # YAML frontmatter parsing
cargo add wiremock --dev       # Mock HTTP server for integration tests
```

No other new dependencies. All other functionality uses existing crates
(reqwest, serde_json, tokio, crossterm, rusqlite, regex, clap, chrono, toml).

---

## File Count Estimate

- New Rust source files: ~4 (profiles.rs, autonomous.rs, bundled_skills.rs, integration tests)
- Modified Rust source files: ~12
- New skill files: ~14
- New model profile files: ~5
- New documentation files: ~3 (MANUAL.md, LESSONS_LEARNED.md, updated README.md)
- New tests: ~25+
- Total estimated new/modified lines: ~3,000-4,000

---

## Implementation Status

*Updated after implementation.*

### Completed (v3.0)

| Requirement | Status | Notes |
|-------------|--------|-------|
| R1: Multi-backend (Ollama, llama-server, MLX, custom) | ✅ Done | `BackendKind` enum, `/backend` command, per-backend auto-detect |
| R2: Model profiles in `.anvil/models/` | ✅ Done | 5 bundled profiles (Qwen3-Coder, Qwen3, Devstral, DeepSeek-R1, GLM-4.7-Flash) |
| R3: Structured skill frontmatter | ✅ Done | YAML frontmatter with category, tags, env, verify. Backward compatible |
| R4: Curated skill pack (14 skills) | ✅ Done | Infrastructure (5), dev tools (4), meta (5) |
| R5: Autonomous mode (Ralph Loop) | ✅ Done | `--autonomous --verify`, iteration/token/time limits, `[ANVIL:DONE]` marker |
| R6: Per-skill env passthrough | ✅ Done | Skills declare env vars in frontmatter, passed to shell when active |
| R7: Documentation | ✅ Done | Doc comments on all public items, MANUAL.md, LESSONS_LEARNED.md, context.md self-prompt |
| R7.7: Self-prompt in context.md | ✅ Done | Lessons learned injected into every session |
| R8: Testing | ✅ Done | 91 tests (up from 66), zero clippy warnings |

### Deferred to v4

| Feature | Reason |
|---------|--------|
| Backend lifecycle management | Connect-only is sufficient for now. Users start servers manually |
| Anthropic/Google API support | All backends must be OpenAI-compatible. Use LiteLLM as proxy if needed |
| Context compaction (`/clear`) | Placeholder exists. Needs token-aware summarization strategy |
| Integration tests with mock HTTP server | Unit tests cover the logic. Mock server adds complexity |
| `/ralph` in interactive mode | CLI `--autonomous` works. Interactive Ralph Loop needs async ownership refactor |
| Ctrl+C cancellation | CancellationToken infrastructure exists but not fully wired |

### Known Issues

1. **Qwen3 thinking mode**: Qwen3 models support `<think>` blocks for chain-of-thought.
   Anvil doesn't parse or hide these — they appear in output. May confuse tool calling.
2. **DeepSeek-R1 reasoning tokens**: Similar to Qwen3, R1 outputs `<think>` blocks.
   These consume context window but aren't useful for tool calling decisions.
3. **MLX server compatibility**: `mlx_lm.server` tool calling support varies by model.
   Not all models expose tool calling through MLX's OpenAI-compatible endpoint.
4. **Large context on Ollama**: Ollama defaults to 2048 context. Users must set
   `OLLAMA_NUM_CTX` or use `/set parameter num_ctx 16384` for larger windows.
