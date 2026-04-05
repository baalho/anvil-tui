# anvil-tui â€” Devil's Advocate Review

**Repository:** [baalho/anvil-tui](https://github.com/baalho/anvil-tui)  
**Version reviewed:** v3.0.0 @ `29eb962` (April 5, 2026)  
**Review date:** April 5, 2026  
**Methodology:** Adversarial code-level analysis across 8 dimensions. Every finding is grounded in specific files, commit SHAs, or API responses. The goal is to surface what could go wrong, not what's working.

---

## Executive Summary

anvil-tui is a Rust-based terminal coding agent for local LLMs (Ollama, llama-server, MLX). It went from initial commit to v3.0.0 in **6 calendar days** (March 30 â€“ April 5, 2026), shipping 22 commits, 6 crates, ~398 tests, 22 bundled skills, daemon/watch modes, model routing, structured table output, Kitty image rendering, and Zellij pane integration. The v3.0.0 release shipped 6 cross-cutting features in a single commit.

**Verdict:** The codebase shows genuine engineering ambition and surprisingly coherent architecture for its age. However, it has **zero users**, **zero community participation**, **significant security surface with no hardening**, **no release infrastructure**, and a **development velocity that outpaces validation**. It is a well-built car with no road to drive on.

**Severity distribution:**  
ðŸ”´ Critical (blocks adoption): 5 findings  
ðŸŸ¡ High (blocks growth): 12 findings  
ðŸŸ¢ Notable (worth tracking): 15 findings  

---

## Milestone 1: Development Velocity & Process Red Flags

### Evidence

| Metric | Value | Source |
|--------|-------|--------|
| First commit | `d0368da` â€” Mar 30, 2026 18:08 UTC | GitHub API |
| v3.0.0 commit | `29eb962` â€” Apr 5, 2026 02:20 UTC | GitHub API |
| Total commits | 22 | `GITHUB_LIST_COMMITS` (full list) |
| Total branches | 1 (`main`) | `GITHUB_LIST_BRANCHES` |
| Git tags | 0 | Repository metadata (topics: []) |
| GitHub Releases | 0 | `release.yml` never triggered |
| Pull requests | 0 | Repository metadata |

### Findings

**ðŸ”´ 1.1 â€” Six features shipped atomically with no integration window.**  
Commit `29eb962` lands model routing (agent.rs), skill search (skills.rs), daemon+watch fusion (daemon.rs), structured output (executor.rs), Kitty images (render.rs), and Zellij panes (zellij.rs) in a single commit. These features interact: model routing changes the turn loop that structured output depends on; daemon+watch creates the long-running process that Zellij panes render within. No feature branch, no incremental merge, no integration testing between features.

**ðŸŸ¡ 1.2 â€” No branching strategy â€” confirmed as policy.**  
The repository has exactly one branch: `main`. Branch protection is `enabled: false`. All 22 commits land directly on main. The user explicitly stated "Single branch: main only. No stale branches." For a tool that executes arbitrary shell commands and writes files, zero code review is a governance gap.

**ðŸŸ¡ 1.3 â€” Dual-identity commits with corporate email raise IP questions.**  
Commits split between two identities:

| Identity | Email | Commits | Style |
|----------|-------|---------|-------|
| `baalho` / `Apaht` | `amolbthapa@gmail.com` | 12 (early) | Terse: "clean up push", "v1", "push" |
| `thapaa4_roche` / `Thapa, Amol {DOBC~INDIANAPOLIS}` | `amol.thapa+roche@roche.com` | 10 (later, all v1.2+) | Detailed multi-paragraph changelogs |

Every `thapaa4_roche` commit carries `Co-authored-by: Ona <no-reply@ona.com>`. Roche is a pharmaceutical company. This raises:
- Is this personal IP or corporate IP?
- Does Roche's employment agreement cover side projects developed with corporate AI tools?
- The Apache-2.0 license is from the personal `baalho` account, but the majority of feature code is from the corporate identity.

**ðŸŸ¡ 1.4 â€” Zero git tags across 3 major versions.**  
The project claims versions 0.1 through 3.0. Zero git tags exist. `release.yml` has never triggered. Users cannot:
- Pin to a specific version via `cargo install`
- Rollback from v3.0 to v2.2 if breaking changes bite
- Verify they're running the version they think they are (beyond `Cargo.toml` strings)

**ðŸŸ¢ 1.5 â€” ADVENTURE.md is manual QA disguised as onboarding.**  
A "7-chapter hands-on walkthrough with a scorecard checklist" is a manual test plan. Its existence implicitly acknowledges that automated tests don't cover feature interactions.

**ðŸŸ¢ 1.6 â€” Version claims are unverifiable.**  
`Cargo.toml` says `version = "3.0.0"` but there's no tag, no release, and no binary artifact. The version is a string in a TOML file, not a release.

---

## Milestone 2: Security & Sandbox Audit

### Evidence

| Attack surface | File | Current state |
|---------------|------|---------------|
| Shell execution | `executor.rs` | `sh -c <string>`, user's full permissions |
| File write | `executor.rs` | Any path the user has access to |
| Kids sandbox | `executor.rs` | Allowlist-based, no OS-level isolation |
| Daemon socket | `daemon.rs` | UDS at `/tmp/anvil-$UID/daemon-<hash>.sock`, 0600 |
| Daemon IPC | `ipc.rs` | Plaintext JSON, no auth token |
| Model routing | `agent.rs` | `/route shell qwen3:8b` mid-conversation |
| File watcher | `daemon.rs` | `DaemonTask::FileChanged` triggers agent runs |
| Zellij subprocess | `zellij.rs` | `Command::new("zellij")` with shell-escaped content |
| Kitty protocol | `render.rs` | Chunked base64 escape sequences to terminal |
| Dependabot | repo settings | `dependabot_security_updates: disabled` |

### Findings

**ðŸ”´ 2.1 â€” Shell execution with auto-approve is encouraged.**  
The README demonstrates `anvil run -p 'fix the build' -y`. The `-y` flag auto-approves ALL tool calls, including `shell`. A hallucinating local LLM (which local models are more prone to than cloud models) can execute arbitrary commands with the user's full permissions. No cgroups, no namespaces, no seccomp, no chroot. The permission system exists but the documentation teaches users to bypass it.

**ðŸ”´ 2.2 â€” Daemon+Watch creates a persistent attack surface triggered by filesystem writes.**  
`anvil daemon start --watch` creates a long-running process that:
1. Monitors file changes via `notify`
2. Triggers `DaemonTask::FileChanged` â†’ `dispatch_event()` â†’ `agent.turn()` for each change
3. The agent then decides what to do, potentially executing shell commands

An attacker who can write to the watched directory can trigger arbitrary agent runs. Combined with `-y` auto-approve, this is a persistent code execution backdoor triggered by filesystem events.

**ðŸŸ¡ 2.3 â€” Model routing enables capability downgrade attacks.**  
`/route shell qwen3:8b` switches to a smaller model for shell tool calls. Smaller models are demonstrably more susceptible to prompt injection and hallucination. Routing to a weaker model mid-task could produce more dangerous tool calls. The routing intentionally doesn't change sampling params, so the weaker model runs with the same tool permissions as the stronger one.

**ðŸŸ¡ 2.4 â€” Zellij pane spawning passes user content through shell.**  
In `zellij.rs`, `open_floating_pane()` constructs:
```rust
let script = format!("echo {} \\| less -R", shell_escape(content));
```
Then passes it to `sh -c`. The `shell_escape()` function wraps content in single quotes and escapes embedded single quotes. While this is the standard approach, any bug in `shell_escape()` (e.g., handling of null bytes, control characters, or very long content) is a shell injection vector. The `open_pane_with_file()` alternative avoids this but is not always used.

**ðŸŸ¡ 2.5 â€” Kitty graphics protocol sends raw escape sequences to terminal emulators.**  
If the LLM instructs the agent to display a crafted file as an image, malformed escape sequences could exploit terminal emulator vulnerabilities. Terminal escape injection is a [known attack class](https://www.usenix.org/conference/usenixsecurity21/presentation/staicu).

**ðŸŸ¡ 2.6 â€” No SECURITY.md or vulnerability disclosure process.**  
The attack surface has expanded significantly with v3.0 (daemon, watch, Zellij, Kitty, model routing). There is no way to responsibly report security issues. No `SECURITY.md`, no security email, no security policy.

**ðŸŸ¡ 2.7 â€” Dependabot security updates explicitly disabled.**  
Repository settings confirm `dependabot_security_updates: disabled`. With 22 direct dependencies including `reqwest`, `rusqlite`, `tokio`, `crossterm`, and `notify`, known CVEs won't be flagged automatically. No `cargo audit` in CI either.

**ðŸŸ¢ 2.8 â€” Kids sandbox is security theater.**  
The kids sandbox uses a command allowlist and path validation. There's no OS-level isolation. Any allowed command can access system resources. `cat /etc/passwd` works if `cat` is allowed. The sandbox gives parents a false sense of safety.

**ðŸŸ¢ 2.9 â€” Daemon IPC has no authentication.**  
The UDS socket has 0600 permissions, but any process running as the same user can send arbitrary JSON to the daemon, including `Request::Shutdown` or `Request::Prompt` with malicious content.

---

## Milestone 3: Architecture & Code Quality Deep Dive

### Evidence

| File | Size (v2.2 est.) | Key concerns |
|------|-----------|--------------|
| `agent.rs` | 47KB + routing logic | God file: agent loop, routing, tools, sessions |
| `commands.rs` | 47KB+ | God file: all slash commands |
| `interactive.rs` | 50KB+ | God file: interactive mode loop |
| `main.rs` | 42KB+ | God file: CLI parsing, initialization |
| `render.rs` | expanded | TerminalCapabilities, tables, Kitty, Zellij delegation |
| `executor.rs` | expanded | `build_table_output()` text parsing |
| `zellij.rs` | ~120 LOC | New module, clean scope |

### Findings

**ðŸŸ¡ 3.1 â€” God files have grown, not shrunk.**  
v3.0.0 added routing logic to `agent.rs`, `build_table_output()` to `executor.rs`, TerminalCapabilities + Kitty protocol + table rendering to `render.rs`, and Zellij delegation from `render.rs`/`commands.rs`. The already-large files absorbed more features. Only `zellij.rs` was extracted as a clean new module.

**ðŸŸ¡ 3.2 â€” Structured output parsing is inherently brittle.**  
`build_table_output()` parses text output of `ls`, `grep`, and `find`:

```rust
// ls parsing assumes format: "dir name/" or "file name (size)"
let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
```

This assumes anvil's *own* `ls` tool output format, not system `ls`. But the parsing is still fragile:
- It assumes exactly two whitespace-delimited fields
- File size extraction uses `rsplit_once(" (")` â€” a filename containing ` (` breaks this
- grep parsing splits on `:` â€” binary file match output like `Binary file foo matches` breaks the 3-field assumption
- find parsing is just `{"path": line.trim()}` â€” robust but trivial

The design decision to wrap in the executor rather than the tools is clever, but it creates a coupling between tool output format and parser expectations that will break silently on edge cases.

**ðŸŸ¡ 3.3 â€” Model routing adds hidden state to the agent loop.**  
The routing logic in `turn()` saves the original model, switches to the routed model for tool-result follow-ups, then restores. This is mutable state threaded through an already-complex loop. Questions:
- If `turn()` errors mid-routing, does the model revert?
- If the daemon restarts during a routed request, which model is active on resume?
- `AgentEvent::ModelSwitched` is emitted but `ModelRouter` is not persisted in `SessionSnapshot` â€” routes are session-ephemeral.

**ðŸŸ¡ 3.4 â€” The 'TUI' in the name is a lie.**  
The project is called `anvil-tui` but uses readline input, not a full-screen TUI framework. The ratatui TUI was explicitly deleted in v1.3 (781 lines of dead `app.rs`). The name creates false expectations.

**ðŸŸ¢ 3.5 â€” The Renderer trait is no longer premature â€” but it's overloaded.**  
v3.0 implements images and tables, validating the Renderer trait's existence. But it now handles plain text, thinking blocks, tool results (text vs table), images (Kitty protocol), and Zellij pane delegation â€” five rendering modes behind one abstraction.

**ðŸŸ¢ 3.6 â€” TerminalCapabilities detection via env vars is unreliable.**  
Detection checks `$TERM_PROGRAM` for Kitty/WezTerm/iTerm2. Known gaps:
- SSH sessions don't forward `$TERM_PROGRAM`
- tmux/screen/Zellij strip or override it
- Some terminals support Kitty graphics without setting the expected var
The "config override path documented for edge cases" means every edge case requires manual configuration.

**ðŸŸ¢ 3.7 â€” Achievement system and inventory system still present.**  
v3.0 doesn't mention removing these v1.x features. The gamification system (achievements.json) and the infrastructure deployment system (inventory.toml with SOPS/age secrets) remain alongside 6 new features, widening scope further.

**ðŸŸ¢ 3.8 â€” `DefaultHasher` for socket path hashing is not stable across Rust versions.**  
`socket_path()` hashes the workspace path with `DefaultHasher`, which is not guaranteed to produce the same hash across Rust compiler versions. A toolchain update could orphan existing daemon sockets.

---

## Milestone 4: Competitive Positioning & Market Viability

### Evidence

| Competitor | Stars | Local model support | Terminal-based | Language |
|-----------|-------|-------------------|---------------|----------|
| Aider | 30K+ | âœ… Ollama, local OpenAI API | âœ… | Python |
| Claude Code | N/A (Anthropic) | âŒ Cloud only | âœ… | TypeScript |
| Cline | 40K+ | âœ… Ollama, local models | âœ… (VS Code) | TypeScript |
| OpenCode | 5K+ | âœ… 75+ providers | âœ… | Go |
| Goose | 15K+ | âœ… Multiple providers | âœ… | Python |
| Codex (OpenAI) | N/A | âŒ Cloud only | âœ… | TypeScript |
| **anvil-tui** | **2** | âœ… Ollama, llama-server, MLX | âœ… | Rust |

Source: [Tembo 2026 comparison](https://www.tembo.io/blog/coding-cli-tools-comparison), [PackmindHub matrix](https://github.com/PackmindHub/coding-agents-matrix), GitHub API

### Findings

**ðŸ”´ 3.1 â€” Zero market awareness in a 24+ tool field.**  
None of the 6 competitive comparison articles found mention anvil-tui. The [PackmindHub matrix](https://github.com/PackmindHub/coding-agents-matrix) compares 24+ agents â€” anvil is absent. With 2 stars and 0 forks, the project is invisible.

**ðŸŸ¡ 4.2 â€” "Local-first" is not a differentiator.**  
Aider has first-class Ollama support with [dedicated documentation](https://aider.chat/docs/llms/ollama.html). Multiple guides exist for running Aider with local models. OpenCode supports 75+ providers. Cline supports Ollama. The offline/airgapped value proposition is already served by established tools with vastly larger user bases.

**ðŸŸ¡ 4.3 â€” No cloud model support eliminates 95%+ of potential users.**  
Every major competitor supports both cloud and local models. anvil-tui supports ONLY local models (Ollama, llama-server, MLX). This is positioned as a feature ("offline-first") but makes it impossible for users to start with cloud models and migrate to local ones â€” a common adoption path.

**ðŸŸ¡ 4.4 â€” Rust raises the contribution barrier.**  
The terminal agent space is dominated by Python (Aider, Goose), Go (OpenCode), and TypeScript (Cline, Claude Code). Choosing Rust limits the contributor pool. The target audience (developers using local LLMs) are disproportionately Python/JS developers.

**ðŸŸ¡ 4.5 â€” v3.0 features target power users exclusively.**  
Every v3.0 feature narrows the audience:
- Kitty graphics: works in ~3 terminals
- Zellij panes: requires Zellij (niche multiplexer)
- Model routing: requires multiple local models
- Structured tables: cosmetic improvement to `ls`/`grep`/`find` output

None of these features help acquire new users. They all assume an existing power user who has already chosen anvil over 24 alternatives.

**ðŸŸ¢ 4.6 â€” No benchmarks or measurable claims.**  
Zero SWE-bench scores, no task completion rates, no speed comparisons. The project makes no measurable claims about quality. Competitors publish benchmarks.

**ðŸŸ¢ 4.7 â€” Feature velocity without users is a warning sign.**  
398 tests, 22 skills, 6 new features, and an audience of approximately one. This is a passion project being developed faster than it can be validated.

---

## Milestone 5: Testing & Reliability Assessment

### Evidence

| Metric | Value | Source |
|--------|-------|--------|
| Total tests | ~398 | User report, v3.0.0 commit message |
| Known failures | 3 (devcontainer detection) | AGENTS.md, commit messages |
| CI test command | `cargo test` (no flags) | `.github/workflows/ci.yml` |
| Mock LLM | None | No wiremock usage despite dependency |
| Fuzzing | None | No proptest/quickcheck/cargo-fuzz |
| Coverage tool | None | No tarpaulin/llvm-cov in CI |
| Flaky test handling | None | No retries in CI |
| CI platforms | ubuntu-latest, macos-14, windows-latest | `ci.yml` matrix |

### Findings

**ðŸŸ¡ 5.1 â€” ~61 new tests for 6 features averages ~10 per feature.**  
Test count went from ~337 to ~398. Model routing state management, daemon+watch lifecycle, Kitty protocol chunking, structured output parsing, and Zellij pane spawning each have far more than 10 testable scenarios. The question is whether new tests cover happy paths only or also error paths and feature interactions.

**ðŸŸ¡ 5.2 â€” Zellij module is untestable in CI.**  
`zellij.rs` has 4 tests â€” all verify behavior *outside* Zellij (availability returns false, shell escape works, pane returns false). The actual Zellij integration (does a pane open? does content display correctly?) requires a running Zellij session that CI doesn't have. The entire pane rendering path is untested automatically.

**ðŸŸ¡ 5.3 â€” Structured output parsing untested against real-world variance.**  
`build_table_output()` parses `ls`, `grep`, and `find` output with format assumptions. No tests verify:
- Filenames containing ` (` (breaks ls size extraction)
- Binary file matches in grep output
- Permission denied errors in find output
- BSD ls vs GNU ls differences
- Locale-dependent date formats

**ðŸŸ¡ 5.4 â€” No mock LLM infrastructure for the most critical path.**  
`wiremock = "0.6"` is in workspace dependencies but the most critical code path â€” model routing during the turn loop (send prompt â†’ parse streaming SSE â†’ extract tool calls â†’ route to different model â†’ execute â†’ loop) â€” has no automated test coverage. Testing model routing requires either a real LLM or a mock server.

**ðŸŸ¡ 5.5 â€” 3 known-failing tests persist across major versions.**  
The devcontainer detection failures existed in v2 and remain in v3. These tests likely fail in CI (GitHub Actions runs in containers). They're either `#[ignore]`d or silently broken. Neither is documented as GitHub issues (0 issues exist).

**ðŸŸ¢ 5.6 â€” CI runs `cargo test` without any retry or coverage flags.**  
No `--retry`, no `--nocapture`, no coverage collection, no test result reporting. Any intermittent failure blocks the pipeline. No code coverage measurement exists.

**ðŸŸ¢ 5.7 â€” ADVENTURE.md compensates for automation gaps.**  
The 7-chapter walkthrough with a scorecard is manual QA. Its existence is an honest acknowledgment that automated tests can't cover the full feature matrix â€” but it's also a sign that the test infrastructure isn't keeping up with feature velocity.

---

## Milestone 6: Documentation & Onboarding Gap Analysis

### Evidence

| Document | Purpose | Concern |
|----------|---------|---------|
| README.md | Intro + install | macOS-centric, no pre-built binary |
| MANUAL.md | Full reference (~24KB) | Assumes expert knowledge |
| AGENTS.md | AI agent instructions (~15KB) | Not for humans |
| CHANGELOG.md | Version history (~20KB) | Date inconsistencies |
| CONTRIBUTING.md | Contributor guide (~2KB) | Minimal |
| ADVENTURE.md | Feature walkthrough (new) | Manual QA disguised as docs |

### CHANGELOG Date Inconsistencies (verified from source)

| Version | Listed Date | Actual Commit Date | Discrepancy |
|---------|------------|-------------------|-------------|
| v2.2.0 | **2025-07-25** | 2026-04-04 | **9 months off, wrong year** |
| v2.1.0 | **2025-07-17** | 2026-04-03 | **9 months off, wrong year** |
| v2.0.0 | **2025-07-17** | 2026-04-03 | **9 months off, wrong year** |
| v1.9.0 | **2025-07-17** | 2026-04-03 | **9 months off, wrong year** |
| v1.8.0 | **2025-07-17** | 2026-04-03 | **Same** |
| v1.7.0 | **2025-07-17** | 2026-04-02 | **Same** |
| v1.6.0 | 2026-04-02 | 2026-04-02 | âœ… Correct |
| v1.1.0 | **2025-04-01** | 2026-04-01 | **Wrong year** |
| v1.0.0 | **2025-03-31** | 2026-03-31 | **Wrong year** |
| v0.1.0 | **2025-03-30** | 2026-03-30 | **Wrong year** |

### Findings

**ðŸ”´ 6.1 â€” CHANGELOG dates are systematically wrong.**  
Every version except v1.2â€“v1.6 has the wrong year (2025 instead of 2026). Versions v1.7â€“v2.2 share the same date (2025-07-17) despite being committed on different days. This is not a typo â€” it's either fabricated retroactively or generated by a tool that defaulted to the wrong year. This fundamentally undermines the changelog as a trust signal.

**ðŸŸ¡ 6.2 â€” No quickstart works in under 30 minutes.**  
Onboarding requires: install Ollama â†’ download a 19GB model â†’ install Rust toolchain â†’ `cargo build --release` (lengthy). No pre-built binary, no Docker image, no `cargo install` (not on crates.io). For v3.0, users also need Kitty/WezTerm/iTerm2 for images and Zellij for panes.

**ðŸŸ¡ 6.3 â€” Documentation volume suggests AI generation, not experience.**  
~67KB+ of documentation for a 6-day-old project is extraordinary. The commit messages from `thapaa4_roche` are multi-paragraph structured changelogs. Combined with the `Ona` co-author on every commit, this documentation reads like it was generated alongside the code by AI, not distilled from real user experience.

**ðŸŸ¡ 6.4 â€” No published rustdoc.**  
6 crates with new public APIs (`TerminalCapabilities`, `ZellijPanes`, `build_table_output`) and no published API documentation. `cargo doc --no-deps` runs in CI with `-D warnings` but the output is discarded. No GitHub Pages site.

**ðŸŸ¢ 6.5 â€” Design decisions documented in chat, not in code.**  
Important rationale ("structured output wraps in the executor, not the tools", "routing doesn't switch sampling params", "capabilities detection is env-var only") exists in chat messages but may not be in code comments or architecture docs. These decisions will be lost to future contributors.

**ðŸŸ¢ 6.6 â€” Feature degradation behavior is undocumented for users.**  
Tables, images, and Zellij panes all degrade gracefully in code (silent fallback). But is the user told what they're missing? Running in a non-Kitty terminal silently skips images. Users may not know the feature exists.

---

## Milestone 7: Dependency Health & Supply Chain Risk

### Evidence (from `Cargo.toml` workspace)

| Dependency | Version | Risk |
|-----------|---------|------|
| `rusqlite` | 0.33, `bundled` | Compiles SQLite from C source |
| `reqwest` | 0.12, `rustls-tls` | `rustls` has had CVEs |
| `notify` | 7.0 | Now critical path for daemon+watch |
| `crossterm` | 0.28 | Still present despite no TUI |
| `libc` | 0.2 | Unix-specific, `#[cfg(unix)]` |
| `wiremock` | 0.6 | Present but unused for LLM mocking |
| `base64` | (new, explicit) | Was transitive, now direct |
| `tempfile` | 3 | Used by zellij.rs |

| CI Check | Present? |
|----------|----------|
| `cargo audit` | âŒ |
| `cargo deny` | âŒ |
| Coverage (`tarpaulin`/`llvm-cov`) | âŒ |
| Dependabot security | âŒ (explicitly disabled) |

### Findings

**ðŸŸ¡ 7.1 â€” `notify` v7.0 is now critical path with known platform bugs.**  
In v3.0, `anvil daemon start --watch` fuses file watching into the daemon. File watcher reliability is now a core feature. `notify` has documented platform-specific issues: macOS FSEvents race conditions, Linux inotify watch limits on large repos, Windows ReadDirectoryChangesW edge cases. These are now user-facing reliability concerns.

**ðŸŸ¡ 7.2 â€” No supply chain auditing in CI.**  
No `cargo audit`, no `cargo deny`, no Dependabot. With 22 direct dependencies, the supply chain surface is significant. Known vulnerabilities in transitive dependencies won't be detected until they cause a user-visible exploit.

**ðŸŸ¡ 7.3 â€” `crossterm` 0.28 remains despite TUI deletion.**  
The TUI was deleted in v1.3. v3.0 uses Kitty escape sequences and box-drawing characters. Is crossterm still needed for terminal manipulation (raw mode, cursor movement), or is it vestigial from the deleted TUI? If vestigial, it's unnecessary binary bloat and attack surface.

**ðŸŸ¢ 7.4 â€” `opt-level = "z"` with `lto = true` + `codegen-units = 1` + `strip = true`.**  
Optimizing for binary size over speed. With 6 new features, release builds are increasingly slow to compile. The size optimization is increasingly questionable as the binary grows.

**ðŸŸ¢ 7.5 â€” MSRV 1.75 (December 2023) is 2+ years old.**  
Conservative MSRVs are good for compatibility, but as dependencies update to require newer Rust features, version conflicts will emerge. No MSRV check in CI verifies this still works.

**ðŸŸ¢ 7.6 â€” Zellij is an unpinned runtime dependency.**  
`zellij.rs` spawns `zellij action` as a subprocess. No version check. If Zellij changes its CLI in a future release, the integration breaks silently (best-effort fallback).

**ðŸŸ¢ 7.7 â€” `wiremock` 0.6 is a dependency but unused for its intended purpose.**  
The dependency exists for HTTP mocking but there's no mock LLM server in tests. The most critical code path (streaming LLM interaction) has no mock infrastructure.

---

## Milestone 8: Community Readiness & Sustainability Assessment

### Evidence

| Metric | Value | Source |
|--------|-------|--------|
| Stars | 2 | GitHub API |
| Forks | 0 | GitHub API |
| Open issues | 0 | GitHub API |
| Closed issues | 0 | GitHub API |
| Pull requests (all time) | 0 | GitHub API |
| Contributors | 1 (two identities) | Commit log |
| Topics/tags | [] (empty) | GitHub API |
| Homepage | "" (empty) | GitHub API |
| Discussions | disabled | GitHub API |
| crates.io publication | âŒ | Not published |
| Code of Conduct | âŒ | Not found |
| SECURITY.md | âŒ | Not found |
| License file | âŒ (license: null in API) | GitHub API shows null |
| CLA | âŒ | Not found |

### Findings

**ðŸ”´ 8.1 â€” The project has zero users.**  
2 stars (likely the author and one acquaintance), 0 forks, 0 issues, 0 PRs, 0 mentions in any competitive comparison. 398 tests and 6 features for an audience of approximately one. No community channel exists (no Discord, Matrix, Discussions, or mailing list).

**ðŸŸ¡ 8.2 â€” Single-contributor bus factor with expanding codebase.**  
One person maintains 6 crates, ~398 tests, 22 skills, daemon mode, model routing, image rendering, Zellij integration, and 67KB+ of documentation. If the maintainer loses interest, the project dies immediately. The v3.0 feature expansion compounds this.

**ðŸŸ¡ 8.3 â€” Corporate email provenance without IP clarification.**  
10 of 22 commits (all feature commits from v1.2 onward) use a Roche corporate email with an AI co-author. The Apache-2.0 license grants patent rights, but if Roche asserts IP ownership over code developed on corporate infrastructure, the license could be contested.

**ðŸŸ¡ 8.4 â€” Not published to crates.io.**  
`cargo install anvil-tui` doesn't work. The standard Rust distribution channel is unused. This is a significant adoption barrier for the Rust community.

**ðŸŸ¢ 8.5 â€” No Code of Conduct or contributor infrastructure.**  
No CoC, minimal CONTRIBUTING.md ("fork and create a feature branch"), no issue templates in use (0 issues filed despite templates existing), no CLA. The project isn't set up for community participation.

**ðŸŸ¢ 8.6 â€” Feature complexity deters first-time contributors.**  
A new contributor would need to understand: the agent turn loop, model routing state, daemon IPC protocol, file watching, Kitty graphics protocol, Zellij subprocess management, structured output parsing, the skill system, personas, modes, achievements, sessions, memory compaction, and the 5-layer system prompt. The cognitive barrier steepened dramatically in 7 days.

**ðŸŸ¢ 8.7 â€” GitHub topics array is empty.**  
No topics/tags means the project won't appear in GitHub topic-based discovery. Simple fix: add `rust`, `terminal`, `coding-agent`, `ollama`, `local-llm`.

---

## Recommendations (Prioritized)

### Immediate (before any further features)

1. **Create git tags for v1.0, v2.0, v3.0** â€” without tags, versions are meaningless
2. **Fix CHANGELOG dates** â€” the wrong-year dates are a credibility issue
3. **Add SECURITY.md** â€” define vulnerability disclosure process
4. **Enable Dependabot** or add `cargo audit` to CI
5. **Publish to crates.io** â€” the standard Rust distribution channel
6. **Add GitHub topics** â€” `rust`, `terminal`, `coding-agent`, `ollama`, `local-llm`

### Short-term (next 2 weeks)

7. **Add a mock LLM server for integration tests** â€” `wiremock` is already a dependency
8. **Test `build_table_output()` against edge cases** â€” filenames with `(`, binary grep matches, permission denied in find
9. **Document degradation behavior** â€” what does the user see when Kitty/Zellij aren't available?
10. **Clarify IP provenance** â€” add a note about employer IP assignment or switch entirely to personal email

### Strategic (next 1â€“3 months)

11. **Add cloud model support** â€” even as optional, this opens 95% of the market
12. **Rename the project** â€” "anvil-tui" without a TUI is confusing; "anvil" or "anvil-cli" is accurate
13. **Extract god files** â€” `agent.rs`, `commands.rs`, `interactive.rs`, `main.rs` each need decomposition
14. **Publish rustdoc to GitHub Pages** â€” `docs.yml` exists but output is discarded
15. **Create a quickstart with pre-built binaries** â€” the 30-minute onboarding is an adoption killer

---

*This review was conducted by analyzing the complete commit history, source code at `29eb962`, repository metadata, CI configuration, dependency tree, and competitive landscape as of April 5, 2026. All findings are grounded in specific evidence cited inline.*