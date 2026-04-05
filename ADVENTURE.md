# The Anvil v3.0 Feature Adventure

A hands-on tour of every new feature in Anvil v3.0. Follow along in
order — each step builds on the previous one. Estimated time: 15 minutes.

---

## Prerequisites

```bash
# Build Anvil
cargo build

# Start your LLM backend (Ollama example)
ollama serve &
ollama pull qwen3:8b
ollama pull qwen3:0.6b   # small model for routing demo
```

---

## Chapter 1: The Table Awakens

> *Your terminal just got a lot prettier.*

Start Anvil and ask it to list files:

```bash
./target/debug/anvil
```

```
you> list the files in the src directory
```

Watch the output — `ls` results now render as aligned tables with
box-drawing borders:

```
  ┌──────┬────────────┬───────┐
  │ type │ name       │ size  │
  ├──────┼────────────┼───────┤
  │ dir  │ src/       │       │
  │ file │ main.rs    │ 4.2K  │
  └──────┴────────────┴───────┘
```

Now try grep:

```
you> search for "fn main" in the codebase
```

Grep results also render as tables:

```
  ┌──────────────────┬──────┬─────────────────────┐
  │ file             │ line │ text                │
  ├──────────────────┼──────┼─────────────────────┤
  │ src/main.rs      │ 42   │ fn main() {         │
  └──────────────────┴──────┴─────────────────────┘
```

The LLM still sees plain text — tables are a display-only enhancement.

---

## Chapter 2: The Routing Gambit

> *Why use a cannon when a slingshot will do?*

Set up model routing — use a tiny model for simple tools:

```
you> /route shell qwen3:0.6b
you> /route grep qwen3:0.6b
you> /route
```

You'll see your routes listed. Now ask Anvil to do something that
uses the shell:

```
you> run `echo "hello from the small model"` and then explain what happened
```

Watch the status line — you'll see:

```
  [routing: qwen3:8b → qwen3:0.6b]
```

The shell command runs on the small model, then the explanation
switches back to the big model. Clear routes when done:

```
you> /route clear
```

---

## Chapter 3: The Skill Hunter

> *22 skills, but which one has what you need?*

Search for skills by keyword:

```
you> /skill search docker
```

You'll see matching skills with their tags. Try multi-keyword search:

```
you> /skill search code review
```

Both terms must match (AND logic). Try searching by category:

```
you> /skill search infrastructure
```

---

## Chapter 4: The Daemon Watcher

> *Two processes become one.*

Open a second terminal. Start the daemon with file watching:

```bash
# Terminal 2
./target/debug/anvil daemon start --watch --debounce 3
```

You'll see:
```
╭─────────────────────────────────────╮
│  anvil daemon v3.0.0                │
│  watching: enabled (debounce: 3s)   │
╰─────────────────────────────────────╯
```

Check status from another terminal:

```bash
# Terminal 3
./target/debug/anvil daemon status
```

Output includes `watching: yes`. Now send a prompt:

```bash
./target/debug/anvil send "what files are in the current directory?"
```

The daemon processes it. Now edit a file — the watcher will trigger
a turn automatically:

```bash
echo "// test change" >> src/main.rs
```

Watch Terminal 2 — the daemon detects the change and runs a turn.

Stop the daemon:

```bash
./target/debug/anvil daemon stop
```

Revert the test change:

```bash
git checkout src/main.rs
```

---

## Chapter 5: The Image Protocol

> *A picture is worth a thousand tokens.*

This feature works in **Kitty**, **WezTerm**, or **iTerm2**. If you're
in a different terminal, you'll see the fallback (file path display).

Create a test image:

```bash
# Create a tiny PNG (1x1 red pixel)
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\x0f\x00\x00\x01\x01\x00\x05\x18\xd8N\x00\x00\x00\x00IEND\xaeB`\x82' > /tmp/test.png
```

In Anvil's interactive mode, the renderer can display images inline.
The infrastructure is ready — when a tool generates an image file,
the renderer will display it using the Kitty graphics protocol.

To verify detection:

```bash
echo "KITTY_WINDOW_ID: ${KITTY_WINDOW_ID:-not set}"
echo "TERM_PROGRAM: ${TERM_PROGRAM:-not set}"
```

---

## Chapter 6: The Zellij Dimension

> *Your chat loop just got a sidekick.*

This feature requires **Zellij**. Install it if you haven't:

```bash
# macOS
brew install zellij

# Linux
cargo install zellij
```

Start Anvil inside Zellij:

```bash
zellij
# Inside Zellij:
./target/debug/anvil
```

Try the `/pane` command:

```
you> /pane Hello from a floating pane! This text appears in a separate Zellij pane.
```

A floating pane opens with your text. Now trigger long output — when
tool output exceeds 50 lines, it automatically opens in a pane:

```
you> list all files recursively in the crates directory
```

If the output is long enough, you'll see it in both the chat (truncated)
and a floating pane (full content).

---

## Chapter 7: The Grand Finale

> *All features working together.*

Set up the ultimate configuration:

```
you> /route grep qwen3:0.6b
you> /skill search code
you> /skill code-review
```

Now ask Anvil to do something that exercises everything:

```
you> find all Rust files containing "pub fn", then review the code patterns
```

Watch as:
1. `find` results render as a table
2. `grep` routes to the small model (you'll see the routing message)
3. The code review uses the full model
4. If in Zellij, long output opens in floating panes

---

## Scorecard

Check off each feature as you verify it:

- [ ] **Table rendering** — ls/grep/find show box-drawing tables
- [ ] **Model routing** — `/route` switches models, `[routing: ...]` shown
- [ ] **Skill search** — `/skill search` finds skills by keyword
- [ ] **Daemon watch** — `daemon start --watch` detects file changes
- [ ] **Image rendering** — Kitty protocol detected (or fallback shown)
- [ ] **Zellij panes** — `/pane` opens floating pane (or "not in Zellij")

---

## Test Summary

```bash
# Run the full test suite
cargo test

# Expected: ~398 passing, 3 known devcontainer-detection failures
# New tests added: routing (3), skill search (4), tables (5),
#                  capabilities (1), Kitty (1), image fallback (1),
#                  Zellij (4), render_tool_output (2)
```

Congratulations — you've explored every new feature in Anvil v3.0!
