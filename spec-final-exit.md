# Spec: Clean Exit — Master Prompt, AGILE Plan, Documentation Bible

## Problem Statement

Anvil v0.1.0 is feature-complete and builds on both this workspace and WSL 2.
The user is spinning up a fresh environment using the GitHub repo as the sole
source of truth. This workspace will be decommissioned.

Three deliverables are needed for a clean handoff:

1. **AGENTS.md** — A Master Prompt that gives any AI agent (in a fresh session
   with zero prior context) everything it needs to continue building Anvil
   toward v1.0. Must be verified, devil's-advocate reviewed, and self-contained.

2. **AGILE.md** — A complete AGILE project plan with feature-driven milestones
   and user stories from v0.1.1 to v1.0.

3. **Documentation Bible** — Auto-generated from code comments via `cargo doc`,
   deployed to GitHub Pages via CI on every push.

Additionally: `.gitignore` must be hardened, environment-specific files must
not leak, and all v3 work must be committed cleanly.

---

## Current State (Verified)

- **GitHub**: `origin/master` at `0015cb4` (v0.1.0 push)
  - Contains `.devcontainer/` and `.ona/` (environment-specific, should not be public)
  - Will be fixed in the new environment by `.gitignore` + `git rm --cached`
- **Local**: HEAD at `614b9a2` with massive uncommitted v3 changes
  - Updated model profiles (Qwen3-Coder, Devstral, DeepSeek-R1)
  - Gap analysis fixes across all docs
  - Updated `.gitignore`
- **Build**: 91 tests pass, 0 clippy warnings, 0 doc warnings
- **Codebase**: 30 source files, 7,350 lines of Rust, 5 crates
- **Binary**: 13MB release build

### P0: Server Recommendation

- **CPU**: 8 cores (Anvil compiles in ~9s release, tests in ~2s)
- **RAM**: 32GB (sufficient for compilation + Ollama with small models)
- **No API keys needed** — Anvil is local-only, connects to Ollama/llama-server/MLX
- **OS**: Any Linux (Ubuntu 22.04+ recommended) or macOS
- **Toolchain**: Rust 1.75+, git

---

## Requirements

### R1: .gitignore Hardening

**R1.1** — `.gitignore` must exclude:
- `/target` (build artifacts)
- `.devcontainer/` (environment-specific)
- `.ona/` (Ona/Gitpod workspace state)
- `.vscode/`, `.idea/` (IDE config)
- `.DS_Store`, `Thumbs.db` (OS junk)
- `.env`, `.env.*` (secrets)
- `.anvil/` (user-specific harness, created by `anvil init`)
- `*.swp`, `*.swo`, `*~` (editor temp files)

**R1.2** — Commit all v3 work (gap analysis, model updates, doc fixes) on top
of current HEAD. Do NOT rewrite history.

### R2: AGENTS.md (Master Prompt)

The bootstrap document for any AI agent starting a fresh session on this repo.
Must be self-contained — no external context needed.

**R2.1** — Project identity: name, purpose, philosophy, license.

**R2.2** — Architecture: crate dependency graph, data flow, key abstractions.

**R2.3** — Current state: what's built (v0.1.0), what works, what's deferred.

**R2.4** — Code conventions: Rust style, error handling patterns, test patterns,
commit message format.

**R2.5** — Development workflow: how to build, test, add features, write skills.

**R2.6** — Lessons learned: the hard-won knowledge from development (shell strings
not argv, readline over TUI, Option<Agent> pattern, etc.).

**R2.7** — Devil's advocate checklist: questions the agent should ask itself
before making changes (Will this work on macOS? Does this break existing tests?
Is this the simplest solution?).

**R2.8** — AGILE context: current version, next milestone, link to AGILE.md.

**R2.9** — Documentation: how to generate docs, where they live, link to
GitHub Pages.

### R3: AGILE.md (Project Plan)

Feature-driven milestones with user stories. Ship when ready, not on a calendar.

**R3.1** — Version scheme: `v0.MINOR.PATCH`. Minor = feature milestone.
Patch = bug fixes within a milestone. v1.0 = production-ready daily driver.

**R3.2** — Each milestone has:
- Version number and codename
- Theme (one sentence)
- User stories in `As a <role>, I want <goal>, so that <benefit>` format
- Acceptance criteria per story
- Definition of Done for the milestone

**R3.3** — Milestones must cover the gap between v0.1.0 and v1.0:
- Context compaction (the `/clear` placeholder)
- Backend lifecycle management (start/stop llama-server)
- Ctrl+C cancellation
- Interactive Ralph Loop (`/ralph` in interactive mode)
- Qwen3/DeepSeek thinking mode parsing
- Plugin/extension system
- Cross-platform testing (macOS, Linux, Windows/WSL)
- Performance optimization
- Error recovery and resilience
- Community readiness (contributing guide, issue templates)

**R3.4** — Backlog section for ideas that don't fit a milestone yet.

### R4: Documentation Bible (GitHub Pages)

**R4.1** — GitHub Actions workflow (`.github/workflows/docs.yml`) that:
1. Triggers on push to `master`
2. Runs `cargo doc --no-deps --document-private-items`
3. Deploys `target/doc/` to GitHub Pages

**R4.2** — Workspace-level `lib.rs` doc comment or `README.md` that cargo doc
uses as the landing page.

**R4.3** — Every public item already has `///` doc comments (verified: zero
doc warnings with `-D warnings`).

**R4.4** — Add `#![doc = include_str!("../../README.md")]` or equivalent to
the main crate's `lib.rs` so the README appears in the generated docs.

### R5: Clean Commit

**R5.1** — Stage all v3 changes + new files (AGENTS.md, AGILE.md, .github/workflows/docs.yml).

**R5.2** — Do NOT stage `.devcontainer/`, `.ona/`, or any environment-specific files.

**R5.3** — Commit message: `v0.1.0: master prompt, agile plan, doc bible, gap analysis fixes`

**R5.4** — Push to `origin/master`.

---

## Acceptance Criteria

1. `.gitignore` excludes all environment-specific and OS-specific files
2. `AGENTS.md` exists at repo root, is self-contained, and a fresh AI agent
   can read it and immediately understand how to work on Anvil
3. `AGILE.md` exists at repo root with feature-driven milestones and user
   stories from v0.1.1 to v1.0
4. `.github/workflows/docs.yml` exists and would deploy cargo doc to GitHub Pages
5. `cargo test` passes (91 tests)
6. `cargo clippy --all-targets -- -D warnings` passes
7. `cargo doc --no-deps` produces zero warnings
8. `git status` shows clean working tree after commit
9. No `.devcontainer/`, `.ona/`, `.env`, or IDE files in the commit

---

## Implementation Plan

| # | Task | Output |
|---|------|--------|
| 1 | Write AGENTS.md (Master Prompt) | `AGENTS.md` |
| 2 | Write AGILE.md (project plan with user stories) | `AGILE.md` |
| 3 | Create GitHub Actions docs workflow | `.github/workflows/docs.yml` |
| 4 | Add crate-level doc attributes for cargo doc landing page | `crates/anvil/src/main.rs` |
| 5 | Verify build, tests, clippy, cargo doc | terminal output |
| 6 | Stage all changes, commit, push | git |
