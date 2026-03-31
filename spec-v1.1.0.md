# Anvil v1.1.0 — Sparkle Edition

## Problem Statement

Anvil v1.0.0 is feature-complete per the original roadmap (30 user stories,
154 tests, 10 milestones). However, three systemic problems block it from
being a trustworthy daily-driver:

1. **Documentation drift**: AGENTS.md, MANUAL.md, README.md, and embedded
   help files are frozen at v0.1.0 state. 13+ features are implemented but
   undocumented. The repo URL, version number, test count, license reference,
   and "deferred features" list are all wrong. The SSOT is broken.

2. **Integration test gap**: `wiremock` is declared as a dependency but never
   used. Zero integration tests exist for SSE streaming, tool call parsing,
   cancellation, or the agent loop. 14 source modules have zero tests.
   The LLM client (`client.rs`, `stream.rs`) — the most critical code path —
   has no tests at all.

3. **No extensibility beyond the binary**: Anvil can't connect to external
   tool servers. The backlog lists MCP integration, git tools, and other
   capabilities that require a plugin architecture. The second user is a
   7-year-old girl who needs a fun, encouraging experience — not a bare
   terminal prompt.

This spec covers **v1.1.0 "Sparkle Edition"** — a single sprint that fixes
the foundation (docs + tests), adds MCP client support and native git tools,
and introduces a fun mode with character personas and achievements.

---

## Current State (Post v1.0.0)

| Metric | Value |
|--------|-------|
| Tests | 154 (9 + 53 + 12 + 14 + 16 + 6 + 15 + 2 + 27) |
| Clippy warnings | 0 |
| Doc warnings | 0 |
| Crates | 5 (anvil-config, anvil-llm, anvil-tools, anvil-agent, anvil) |
| Source files | ~35 `.rs` files |
| Lines of code | ~10,666 |
| Integration tests with wiremock | **0** |
| Modules with zero tests | **14** |
| AGENTS.md inaccuracies | **13+** |
| Features missing from MANUAL.md | **13** |
| Features missing from README.md | **11** |

---

## Sprint Structure

The sprint has 4 tracks executed in dependency order:

```
Track 1: SSOT Fix (docs)     ──┐
Track 2: Integration Tests    ──┼──► Track 3: New Features ──► Track 4: Fun Mode
                               ──┘
```

Track 1 and 2 can run in parallel. Track 3 depends on Track 2 (test
infrastructure). Track 4 depends on Track 3 (uses the new crate structure).

---

## Track 1: SSOT Fix — Documentation Accuracy

### S31: AGENTS.md Rewrite

**Problem**: AGENTS.md is the single source of truth but contains 13+
inaccuracies. It says v0.1.0, 91 tests, repo `anvil-cli`, and lists
implemented features as "deferred".

**Requirements**:
- S31.1 — Update version to v1.1.0, test count to actual, repo URL to `anvil-tui`
- S31.2 — Remove "What's deferred" section — replace with accurate "Current capabilities"
- S31.3 — Add new modules to Key Files Reference: `routing.rs`, `memory.rs`,
  `thinking.rs`, `plugins.rs`, `hooks.rs`, `migration.rs`, `backend.rs`
- S31.4 — Add new key types to Crate Responsibilities table: `ModelRouter`,
  `ThinkingFilter`, `MemoryStore`, `ToolPlugin`, `HookRunner`, `McpManager`
- S31.5 — Update Architecture section with new crate (`anvil-mcp`)
- S31.6 — Add new Lessons Learned entries for v0.1.1–v1.0.0 patterns
- S31.7 — Update Development Workflow with new "Add an MCP server" guide
- S31.8 — Update Devil's Advocate Checklist with MCP-specific items

**Acceptance Criteria**:
- Every fact in AGENTS.md is verifiable against the codebase
- `system_prompt.rs` loads the updated file without errors

### S32: MANUAL.md Update

**Problem**: MANUAL.md is missing 13 features implemented since v0.1.0.

**Requirements**:
- S32.1 — Add sections for: Ctrl+C cancellation, thinking mode, context
  compaction, interactive Ralph Loop, model routing, parallel tools, cost
  tracking, custom tool plugins, hook system, skill dependencies, project
  memory, session search, config migration
- S32.2 — Update Interactive Commands table with all new slash commands:
  `/compact`, `/think`, `/memory`, `/route`, `/backend start|stop`
- S32.3 — Update Configuration Reference with new settings:
  `auto_compact_threshold`, pricing config, MCP server config
- S32.4 — Add MCP section explaining how to configure and use MCP servers
- S32.5 — Add Fun Mode section explaining personas and achievements
- S32.6 — Update Architecture diagram to include `anvil-mcp` crate

**Acceptance Criteria**:
- Every implemented feature has a corresponding MANUAL.md section
- Configuration examples are copy-pasteable and valid TOML

### S33: README.md + Embedded Help Refresh

**Problem**: README.md says MIT license (should be Apache-2.0), is missing
11 features, and the embedded help files in `crates/anvil/src/help/` are
incomplete.

**Requirements**:
- S33.1 — Fix license to Apache-2.0 in README.md
- S33.2 — Add feature highlights for: MCP, model routing, fun mode,
  thinking mode, context compaction, git tools, achievements
- S33.3 — Update `help/commands.md` with all current slash commands
- S33.4 — Update `help/tools.md` with git tools and MCP tool discovery
- S33.5 — Update `help/config.md` with MCP server configuration
- S33.6 — Add `help/fun.md` explaining personas and achievements
- S33.7 — Update `help/skills.md` with `depends` field documentation
- S33.8 — Update CHANGELOG.md with v1.1.0 entries and actual dates

**Acceptance Criteria**:
- `anvil docs <topic>` shows accurate information for all topics
- README.md license matches `Cargo.toml`

### S34: AGILE.md v1.1.0+ Roadmap

**Problem**: AGILE.md ends at v1.0.0. The backlog needs to become the next
set of milestones.

**Requirements**:
- S34.1 — Mark v0.1.1 through v1.0.0 as "Done" with summary
- S34.2 — Add v1.1.0 Sparkle Edition milestone with stories S35–S42
- S34.3 — Add v1.2.0+ backlog items (vision, LSP, REPL, conversation branching)
- S34.4 — Update LESSONS_LEARNED.md with patterns from v0.1.1–v1.0.0 development

**Acceptance Criteria**:
- AGILE.md reflects the actual project state
- Backlog items have clear descriptions

---

## Track 2: Integration Test Infrastructure

### S35: Wiremock SSE Mock Server

**Problem**: `wiremock` is a dev dependency but has zero usage. The LLM
client (`client.rs`) and SSE parser (`stream.rs`) — the most critical code
paths — have no tests.

**Requirements**:
- S35.1 — Create `crates/anvil-llm/tests/streaming_tests.rs` with wiremock
  mock server that returns SSE-formatted chat completion responses
- S35.2 — Test: single content delta stream → verify `StreamEvent::ContentDelta`
- S35.3 — Test: tool call delta stream (multi-chunk) → verify `ToolCallAccumulator`
  assembles complete tool call with id, name, arguments
- S35.4 — Test: usage stats in final chunk → verify `StreamEvent::Usage`
- S35.5 — Test: mid-stream disconnect → verify `StreamEvent::Error` emitted
- S35.6 — Test: `CancellationToken` cancels stream → verify early termination
- S35.7 — Test: retry on 429 → verify exponential backoff and eventual success
- S35.8 — Test: permanent error (404) → verify no retry, immediate failure
- S35.9 — Create SSE test fixture helper: `fn sse_response(events: &[&str]) -> ResponseTemplate`
  that formats events as `data: {...}\n\n` with proper SSE framing

**Files**: `crates/anvil-llm/tests/streaming_tests.rs`, `crates/anvil-llm/tests/helpers.rs`

**Acceptance Criteria**:
- At least 8 new integration tests using wiremock
- Tests run without network access (fully mocked)
- `cargo test -p anvil-llm` exercises the real HTTP client code path

### S36: Agent Loop Integration Tests

**Problem**: The agent loop (`Agent::turn()`) has unit tests for compaction
range but no integration tests for the full turn cycle.

**Requirements**:
- S36.1 — Create `crates/anvil-agent/tests/agent_loop_tests.rs`
- S36.2 — Test: simple content response → verify message appended to history
- S36.3 — Test: tool call response → verify tool executed and result sent back
- S36.4 — Test: parallel tool calls → verify read-only tools run concurrently
- S36.5 — Test: input validation → verify missing args produce error tool result
- S36.6 — Test: cancellation mid-turn → verify `AgentEvent::Cancelled` emitted
- S36.7 — Use wiremock for the LLM backend, real `ToolExecutor` with `TempDir`

**Files**: `crates/anvil-agent/tests/agent_loop_tests.rs`

**Acceptance Criteria**:
- At least 5 new integration tests for the agent loop
- Tests use real HTTP (wiremock), real tool execution (tempdir), real SQLite

### S37: Tool Executor Tests

**Problem**: `executor.rs`, `permission.rs`, and `tools.rs` have no inline
unit tests. The integration tests in `tool_tests.rs` cover happy paths but
miss edge cases.

**Requirements**:
- S37.1 — Add tests for `ToolExecutor::validate_args()` edge cases:
  numeric args, boolean args, nested objects
- S37.2 — Add tests for file cache: read → cache hit → write invalidates → re-read
- S37.3 — Add tests for `PermissionHandler`: grant persistence, `is_read_only` classification
- S37.4 — Add tests for hook execution: pre-hook blocks, post-hook runs after success
- S37.5 — Add tests for custom plugin execution: template rendering with all arg types

**Files**: `crates/anvil-tools/tests/tool_tests.rs` (extend existing)

**Acceptance Criteria**:
- At least 10 new tests across executor, permission, hooks, and plugins
- File cache behavior verified with assertions on cache size

---

## Track 3: New Features

### S38: MCP Client Support (anvil-mcp crate)

**Problem**: Anvil can't connect to external tool servers. MCP (Model Context
Protocol) is the emerging standard for AI tool integration, with an official
Rust SDK (`rmcp`).

**Requirements**:
- S38.1 — Create new crate `crates/anvil-mcp/` with `McpManager` type
- S38.2 — Add `rmcp` dependency (with `client` feature) to workspace
- S38.3 — `McpManager::new(configs)` connects to configured servers via stdio transport
- S38.4 — Each server connection: spawn child process, run MCP initialization
  handshake, call `tools/list` to discover tools
- S38.5 — `McpManager::tool_definitions()` returns merged tool list (namespaced
  as `mcp_{server}_{tool}` to avoid conflicts with built-in tools)
- S38.6 — `McpManager::call_tool(server, name, args)` dispatches tool calls
  to the correct server via JSON-RPC
- S38.7 — Server lifecycle: start on `McpManager::new()`, graceful shutdown
  on drop (SIGTERM → SIGKILL after 5s, matching Anvil's existing pattern)
- S38.8 — MCP server `instructions` (from init handshake) appended to system prompt
- S38.9 — Configuration in `.anvil/config.toml`:
  ```toml
  [[mcp.servers]]
  name = "filesystem"
  command = "npx"
  args = ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
  env = { NODE_PATH = "/usr/local/lib" }
  ```
- S38.10 — `/mcp` slash command: list connected servers, their tools, and status
- S38.11 — `/mcp restart <name>` reconnects a server
- S38.12 — MCP tools go through `PermissionHandler` (classified as mutating by default)

**Architecture**:
```
anvil-config ──┬──► anvil-llm ──┐
               │                ├──► anvil-agent ──► anvil (binary)
               ├──► anvil-tools ┘
               └──► anvil-mcp (NEW)
```

`anvil-mcp` depends on `anvil-config` (for `McpServerConfig`) and `rmcp`.
`anvil-agent` depends on `anvil-mcp` (to merge tools and dispatch calls).

**Files**:
- New: `crates/anvil-mcp/Cargo.toml`, `crates/anvil-mcp/src/lib.rs`,
  `crates/anvil-mcp/src/manager.rs`, `crates/anvil-mcp/src/config.rs`
- Modified: `Cargo.toml` (workspace), `crates/anvil-agent/Cargo.toml`,
  `crates/anvil-agent/src/agent.rs`, `crates/anvil-config/src/settings.rs`,
  `crates/anvil/src/commands.rs`, `crates/anvil/src/interactive.rs`

**Tests**:
- Unit: Config parsing for MCP server entries
- Unit: Tool name namespacing (`mcp_filesystem_read_file`)
- Unit: Tool definition merging (built-in + MCP)
- Integration: Mock MCP server via stdio (echo server pattern)

**Acceptance Criteria**:
- `anvil` with `[[mcp.servers]]` config connects to servers at startup
- MCP tools appear in LLM's tool list alongside built-in tools
- LLM can call MCP tools and receive results
- `/mcp` shows connected servers and tool counts
- Server crash doesn't crash Anvil (graceful error handling)

### S39: Native Git Tools

**Problem**: Git operations go through the shell tool, which means the LLM
must construct git commands from scratch. Native tools provide structured
input/output and better error handling.

**Requirements**:
- S39.1 — `git_status` tool: returns structured status (staged, modified, untracked)
- S39.2 — `git_diff` tool: returns diff for specified files or all changes.
  Parameters: `path` (optional), `staged` (boolean, default false)
- S39.3 — `git_log` tool: returns recent commits. Parameters: `count`
  (default 10), `path` (optional filter)
- S39.4 — `git_commit` tool: stages specified files and commits.
  Parameters: `message` (required), `files` (array, default all staged)
- S39.5 — All git tools use `tokio::process::Command` with `git` binary
- S39.6 — Git tools are workspace-scoped (run in workspace root)
- S39.7 — `git_status`, `git_diff`, `git_log` classified as read-only
- S39.8 — `git_commit` classified as mutating (requires permission)
- S39.9 — Add JSON schemas to `definitions.rs`
- S39.10 — Add to `executor.rs` dispatch

**Files**:
- Modified: `crates/anvil-tools/src/tools.rs`, `crates/anvil-tools/src/definitions.rs`,
  `crates/anvil-tools/src/executor.rs`, `crates/anvil-tools/src/permission.rs`

**Tests**:
- `git_status` in a temp git repo with staged/unstaged files
- `git_diff` shows correct diff content
- `git_log` returns formatted commit history
- `git_commit` creates a commit with correct message
- All git tools fail gracefully outside a git repo

**Acceptance Criteria**:
- LLM can use `git_status`, `git_diff`, `git_log`, `git_commit` as structured tools
- Git tools produce clean, parseable output
- Permission prompt shown before `git_commit`

### S40: Domain Context Memory

**Problem**: The existing `.anvil/memory/` system stores flat notes. It lacks
structure for short-term (session) vs long-term (project) memory, and has no
automatic learning from conversations.

**Requirements**:
- S40.1 — **Short-term memory**: Key facts from the current session stored in
  a `SessionMemory` struct. Auto-extracted from tool results (e.g., "test
  framework is pytest", "main entry point is src/main.rs")
- S40.2 — **Long-term memory**: Promoted from short-term via `/memory promote`
  or auto-promoted when the same fact appears in 3+ sessions
- S40.3 — Memory categories: `project` (tech stack, conventions), `user`
  (preferences, patterns), `error` (past mistakes to avoid)
- S40.4 — `/memory` shows both short-term and long-term with categories
- S40.5 — `/memory promote <id>` moves a short-term memory to long-term
- S40.6 — Long-term memory injected into system prompt (existing behavior,
  now with categories)
- S40.7 — Short-term memory available as context within the session but not
  persisted to system prompt

**Files**:
- Modified: `crates/anvil-agent/src/memory.rs`, `crates/anvil-agent/src/system_prompt.rs`,
  `crates/anvil/src/commands.rs`

**Tests**:
- Short-term memory stores and retrieves facts
- Long-term memory persists across sessions
- Category filtering works
- `/memory promote` moves between stores

**Acceptance Criteria**:
- Session facts tracked automatically
- `/memory` shows categorized memories
- Long-term memories survive session restart

---

## Track 4: Fun Mode — Sparkle Edition

### S41: Character Personas

**Problem**: Anvil's terminal interface is functional but austere. A 7-year-old
needs encouragement, personality, and delight.

**Requirements**:
- S41.1 — `/persona` command to switch between characters:
  - `default` — standard Anvil (no personality injection)
  - `sparkle` — Sparkle the Coding Fairy ("Let's sprinkle some code magic!")
  - `bolt` — Bolt the Robot ("PROCESSING... SOLUTION COMPUTED!")
  - `captain` — Captain Codebeard the Pirate ("Arrr, let's sail through this bug!")
- S41.2 — Each persona has a system prompt prefix injected before the main
  system prompt. Stored as bundled skills in `bundled_skills.rs`
- S41.3 — Persona affects:
  - Greeting message on session start
  - Tool result announcements (e.g., "Sparkle waves her wand... file written!")
  - Error messages (e.g., "Oh no, a bug dragon appeared! Let's defeat it!")
  - Turn completion messages
- S41.4 — Persona persists in `.anvil/config.toml` under `[agent] persona = "sparkle"`
- S41.5 — Persona-themed color schemes:
  - `sparkle`: magenta/pink accents
  - `bolt`: cyan/blue accents
  - `captain`: yellow/gold accents
- S41.6 — `/persona` with no args shows current persona and available options

**Files**:
- New: `crates/anvil-agent/src/persona.rs`
- Modified: `crates/anvil-config/src/settings.rs`, `crates/anvil-config/src/bundled_skills.rs`,
  `crates/anvil-agent/src/system_prompt.rs`, `crates/anvil/src/interactive.rs`,
  `crates/anvil/src/commands.rs`

**Tests**:
- Persona loading from config
- System prompt includes persona prefix
- Persona switching updates active persona
- Default persona injects nothing

**Acceptance Criteria**:
- `/persona sparkle` changes Anvil's personality immediately
- Persona persists across sessions via config
- A 7-year-old would smile at the output

### S42: Achievement System

**Problem**: Kids (and adults) are motivated by progress indicators and rewards.
Anvil has no way to celebrate accomplishments.

**Requirements**:
- S42.1 — Achievement definitions stored in `crates/anvil-agent/src/achievements.rs`
- S42.2 — Achievements tracked per-project in `.anvil/achievements.json`
- S42.3 — Built-in achievements:
  - "First Steps" — complete your first session
  - "Bug Squasher" — fix a failing test (detected via shell exit code change)
  - "Bookworm" — read 10 files in one session
  - "Speed Demon" — complete a task in under 30 seconds
  - "Explorer" — use all 7 built-in tools in one session
  - "Memory Master" — add 5 memories
  - "Skill Collector" — activate 3 different skills
  - "Ralph Runner" — complete an autonomous loop successfully
  - "Code Fairy" — write 100 lines of code (via file_write/file_edit)
  - "Git Guardian" — make 5 commits using git tools
- S42.4 — Achievement unlocked notification:
  - Default persona: `Achievement unlocked: Bug Squasher`
  - Sparkle: `Sparkle sprinkles confetti! You earned: Bug Squasher!`
  - Bolt: `ACHIEVEMENT UNLOCKED >>> BUG_SQUASHER.exe`
  - Captain: `Ye found treasure! Bug Squasher added to yer chest!`
- S42.5 — `/achievements` command shows all achievements with locked/unlocked status
- S42.6 — Achievement progress persists across sessions
- S42.7 — `AgentEvent::AchievementUnlocked(Achievement)` event for display

**Files**:
- New: `crates/anvil-agent/src/achievements.rs`
- Modified: `crates/anvil-agent/src/agent.rs`, `crates/anvil-agent/src/lib.rs`,
  `crates/anvil/src/interactive.rs`, `crates/anvil/src/commands.rs`

**Tests**:
- Achievement tracking: increment counter, check threshold
- Achievement persistence: save/load from JSON
- Achievement unlocking: event emitted on first unlock, not on subsequent
- All 10 built-in achievements have valid trigger conditions

**Acceptance Criteria**:
- Achievements unlock during normal usage
- `/achievements` shows progress
- Notifications are persona-themed
- A 7-year-old would want to collect them all

### S43: Kid-Friendly Skill Pack

**Problem**: The 14 bundled skills are for professional developers. A young
learner needs age-appropriate coding guidance.

**Requirements**:
- S43.1 — `learn-coding` skill: teaches basic programming concepts with
  simple language, uses analogies (variables = boxes, functions = recipes)
- S43.2 — `story-coder` skill: helps write interactive stories with code
  (Python or Scratch-like pseudocode)
- S43.3 — `art-coder` skill: creates ASCII art and simple graphics with code
- S43.4 — `game-maker` skill: guides building simple text games
  (number guessing, rock-paper-scissors, adventure games)
- S43.5 — All kid skills use encouraging language, celebrate mistakes as
  learning opportunities, and break tasks into tiny steps
- S43.6 — Kid skills have `category: learning` and `tags: [kids, beginner]`
- S43.7 — Kid skills include `verify` commands that test the created programs

**Files**:
- Modified: `crates/anvil-config/src/bundled_skills.rs`

**Tests**:
- All kid skills parse correctly (frontmatter + content)
- Kid skills have required fields (description, category, tags)
- Verify commands are valid shell commands

**Acceptance Criteria**:
- `/skill learn-coding` activates age-appropriate coding guidance
- Skills use simple language a 7-year-old can understand
- Each skill produces a working program the child can run

---

## Track 0: CI Fix (Prerequisite)

### S30.5: Fix CI Workflow Failures

**Problem**: Both GitHub Actions CI runs (Ubuntu and macOS) fail on
`cargo fmt --all -- --check`. The code written in v0.1.1–v1.0.0 was never
formatted with `rustfmt`. ~10 files have formatting violations (line wrapping,
trailing whitespace, import ordering). This blocks all future pushes.

**Root Cause** (from CI logs at run `23811706626`):
- `backend.rs`: line wrapping in `.map_err()` and `eprintln!()`
- `commands.rs`: line wrapping in `format!()`, `.any()` chain, trailing blank line
- `interactive.rs`: line wrapping in `Print(format!())`
- `main.rs`: line wrapping in `println!()`
- `agent.rs`: line wrapping in `if` condition
- `lib.rs`: import ordering (`MemoryStore` before `autonomous`)
- `thinking.rs`: trailing whitespace in doc comment
- `session_tests.rs`: line wrapping in `.save_message()` calls

**Requirements**:
- S30.5.1 — Run `cargo fmt --all` to fix all formatting violations
- S30.5.2 — Verify `cargo fmt --all -- --check` passes (exit 0)
- S30.5.3 — Update CI workflow to the v0.8.0 version (add Windows, use
  `macos-14`, add doc warnings step, add `fail-fast: false`)
- S30.5.4 — Verify `cargo test`, `cargo clippy`, and `cargo doc` still pass
  after formatting

**Acceptance Criteria**:
- `cargo fmt --all -- --check` exits 0
- CI workflow matches the updated version in `.github/workflows/ci.yml`
- All existing 154 tests still pass

---

## Implementation Order

| Step | Track | Stories | Key Deliverable | Est. Tests |
|------|-------|---------|-----------------|------------|
| 0 | T0 | S30.5 | Fix CI (cargo fmt + workflow) | — |
| 1 | T2 | S35 | Wiremock SSE integration tests | +8 |
| 2 | T2 | S36 | Agent loop integration tests | +5 |
| 3 | T2 | S37 | Tool executor edge case tests | +10 |
| 4 | T3 | S38 | MCP client crate (anvil-mcp) | +8 |
| 5 | T3 | S39 | Native git tools | +5 |
| 6 | T3 | S40 | Domain context memory | +4 |
| 7 | T4 | S41 | Character personas | +4 |
| 8 | T4 | S42 | Achievement system | +4 |
| 9 | T4 | S43 | Kid-friendly skill pack | +3 |
| 10 | T1 | S31-S34 | SSOT fix (all docs) | — |

**Rationale**: CI fix first (T0) — nothing else matters if CI is red.
Tests next (T2) so we catch regressions as we add features. Features next
(T3) so docs can reference the final state. Docs last (T1) because they
describe everything else and should be written against the final codebase.

---

## New Dependencies

| Crate | Purpose | Feature |
|-------|---------|---------|
| `rmcp` | Official Rust MCP SDK | `client` |
| `tokio-util` | Already in workspace | — |
| `wiremock` | Already in workspace (dev) | — |

No other new dependencies. `rmcp` brings `jsonrpc-core` transitively.

---

## New Crate

```
crates/anvil-mcp/
├── Cargo.toml
└── src/
    ├── lib.rs          # Re-exports
    ├── manager.rs      # McpManager — multi-server connection manager
    └── config.rs       # McpServerConfig deserialization
```

---

## New Files Estimate

| Category | Count |
|----------|-------|
| New Rust source files | ~8 |
| Modified Rust source files | ~20 |
| New test files | 3 |
| Modified test files | 2 |
| Modified documentation files | 7 |
| New help files | 1 |

---

## Definition of Done (v1.1.0)

1. **CI green**: `cargo fmt --check`, `cargo clippy`, `cargo test`, `cargo doc` all pass
2. All stories S30.5, S31–S43 implemented
3. At least 50 new tests (target: 200+ total)
4. Zero wiremock-unused warnings
5. `cargo clippy --all-targets -- -D warnings` clean
6. `cargo doc --no-deps` clean
7. Every fact in AGENTS.md verifiable against codebase
8. Every implemented feature documented in MANUAL.md
9. README.md license matches Cargo.toml
10. `/persona sparkle` produces a delightful experience
11. `/achievements` shows progress toward 10 built-in achievements
12. At least one MCP server connectable via config
13. Git tools work in any git repository
14. GitHub Actions CI passes on push to main

---

## Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| `rmcp` API instability (v0.16.x) | Breaking changes | Pin exact version, wrap in thin adapter |
| MCP server startup latency | Slow Anvil startup | Connect lazily on first MCP tool call |
| Persona prompts confuse capable models | Degraded coding quality | Persona prefix is short (~50 tokens), tested with qwen3 |
| Achievement tracking overhead | Performance | JSON file, written only on unlock (not every turn) |
| Kid skills too simple for LLM | Model ignores instructions | Test with smallest supported model (qwen3:8b) |
| Git tools in non-git directories | Crashes | Check `.git/` exists, return clear error |

---

## Devil's Advocate Additions

11. **Does the MCP tool name conflict with a built-in?** Namespace all MCP
    tools as `mcp_{server}_{tool}`. Never allow collision.
12. **Will the persona prompt waste context tokens?** Keep persona prefixes
    under 100 tokens. The LLM needs context for code, not character acting.
13. **Can a 7-year-old actually use a terminal?** Yes, with guidance. The
    persona makes errors friendly, achievements provide motivation, and kid
    skills break tasks into tiny steps. The parent (user #1) will be present.
14. **Does the achievement JSON grow unbounded?** No — 10 achievements,
    each with a boolean + counter. Fixed size.
