//! Bundled skill content shipped with `anvil init`.
//!
//! Each skill is a (filename, content) pair. Skills use YAML frontmatter
//! for metadata and markdown for the prompt template. They serve dual purpose:
//! 1. Prompt template — injected into the system prompt when activated
//! 2. Documentation — teaches concepts through examples and explanations

/// All bundled skills, written to `.anvil/skills/` by `init_harness()`.
pub const BUNDLED_SKILLS: &[(&str, &str)] = &[
    // --- Infrastructure ---
    ("docker.md", DOCKER),
    ("docker-compose.md", DOCKER_COMPOSE),
    ("server-admin.md", SERVER_ADMIN),
    ("grafana.md", GRAFANA),
    ("prometheus.md", PROMETHEUS),
    // --- Dev Tools ---
    ("nvim.md", NVIM),
    ("zellij.md", ZELLIJ),
    ("fish.md", FISH),
    ("git-workflow.md", GIT_WORKFLOW),
    // --- Meta ---
    ("verify-all.md", VERIFY_ALL),
    ("verify-shell.md", VERIFY_SHELL),
    ("verify-files.md", VERIFY_FILES),
    ("learn-anvil.md", LEARN_ANVIL),
    ("learn-rust.md", LEARN_RUST),
    // --- Kids ---
    ("kids-first.md", KIDS_FIRST_PROGRAM),
    ("kids-story.md", KIDS_STORYTELLING),
    ("kids-game.md", KIDS_GAME_MAKER),
];

const DOCKER: &str = r#"---
description: "Manage Docker containers, images, volumes, and networks"
category: infrastructure
tags: [docker, containers, devops]
env:
  - DOCKER_HOST
  - DOCKER_CONFIG
  - DOCKER_BUILDKIT
verify: "docker info --format '{{.ServerVersion}}'"
---
# Docker Management

## Concepts
Docker containers are lightweight isolated processes. Images are read-only
templates. Volumes persist data. Networks connect containers.

## Instructions
You are helping manage Docker on this system. Use these patterns:

### Container lifecycle
- `docker ps -a` — list all containers (running and stopped)
- `docker run -d --name <n> <image>` — start detached container
- `docker logs -f --tail 100 <n>` — follow logs, last 100 lines
- `docker exec -it <n> sh` — interactive shell in running container
- `docker stop <n> && docker rm <n>` — clean shutdown and removal

### Image management
- `docker images` — list local images with sizes
- `docker pull <image>:<tag>` — pull specific version
- `docker build -t <name>:<tag> .` — build from Dockerfile
- `docker image prune -f` — remove dangling images

### Volume and network
- `docker volume ls` — list volumes
- `docker volume create <name>` — create named volume
- `docker network ls` — list networks
- `docker network inspect <name>` — show network details

### Debugging
- `docker inspect <container>` — full container metadata as JSON
- `docker stats --no-stream` — one-shot resource usage
- `docker system df` — disk usage breakdown

## Examples
```bash
# Run nginx with port mapping and volume
docker run -d --name web -p 8080:80 -v ./html:/usr/share/nginx/html nginx:alpine

# Check why a container exited
docker logs --tail 50 <container>
docker inspect <container> --format '{{.State.ExitCode}}: {{.State.Error}}'
```
"#;

const DOCKER_COMPOSE: &str = r#"---
description: "Orchestrate multi-container applications with Docker Compose"
category: infrastructure
tags: [docker, compose, orchestration]
env:
  - DOCKER_HOST
  - COMPOSE_FILE
  - COMPOSE_PROJECT_NAME
verify: "docker compose version"
---
# Docker Compose

## Concepts
Compose defines multi-container apps in a YAML file. Services, networks,
and volumes are declared together. Compose v2 is a Docker CLI plugin
(`docker compose` not `docker-compose`).

## Instructions
Help manage Compose stacks. Always use `docker compose` (v2 syntax).

### Stack lifecycle
- `docker compose up -d` — start all services detached
- `docker compose down` — stop and remove containers (keeps volumes)
- `docker compose down -v` — stop and remove containers AND volumes
- `docker compose restart <service>` — restart one service
- `docker compose pull` — pull latest images for all services

### Monitoring
- `docker compose ps` — service status
- `docker compose logs -f <service>` — follow one service's logs
- `docker compose top` — running processes per service

### Configuration
- `docker compose config` — validate and render the final config
- `docker compose --env-file .env.prod up -d` — use specific env file

## Examples
```yaml
# docker-compose.yml
services:
  app:
    build: .
    ports: ["3000:3000"]
    environment:
      DATABASE_URL: postgres://db:5432/app
    depends_on: [db]
  db:
    image: postgres:16-alpine
    volumes: [pgdata:/var/lib/postgresql/data]
    environment:
      POSTGRES_PASSWORD: ${DB_PASSWORD}
volumes:
  pgdata:
```
"#;

const SERVER_ADMIN: &str = r#"---
description: "System administration — services, disks, processes, SSH"
category: infrastructure
tags: [linux, macos, sysadmin, ssh]
env:
  - SSH_AUTH_SOCK
  - KUBECONFIG
verify: "uname -a"
---
# Server Administration

## Concepts
Server admin covers service management, resource monitoring, disk health,
and remote access. macOS uses `launchctl`, Linux uses `systemctl`.

## Instructions
Help with system administration tasks. Detect the OS first.

### Service management (Linux)
- `systemctl status <service>` — check service state
- `systemctl restart <service>` — restart a service
- `journalctl -u <service> -f --since '5 min ago'` — recent logs

### Service management (macOS)
- `launchctl list | grep <name>` — find a service
- `brew services list` — Homebrew-managed services
- `brew services restart <name>` — restart via Homebrew

### Resource monitoring
- `top -l 1 | head -10` (macOS) or `top -bn1 | head -10` (Linux) — snapshot
- `df -h` — disk usage by filesystem
- `du -sh * | sort -rh | head -20` — largest items in current directory
- `free -h` (Linux) or `vm_stat` (macOS) — memory usage
- `lsof -i :<port>` — what's using a port

### Network
- `ss -tlnp` (Linux) or `lsof -iTCP -sTCP:LISTEN` (macOS) — listening ports
- `curl -sI <url>` — HTTP headers only
- `dig <domain>` or `nslookup <domain>` — DNS lookup

### SSH
- `ssh -T git@github.com` — test GitHub SSH
- `ssh-add -l` — list loaded SSH keys
- `ssh -L 8080:localhost:80 user@host` — port forward

## Examples
```bash
# Find what's eating disk space
du -sh /var/log/* | sort -rh | head -10

# Check if a port is in use
lsof -i :8080
```
"#;

const GRAFANA: &str = r#"---
description: "Grafana dashboard provisioning, datasources, and alerting"
category: infrastructure
tags: [grafana, monitoring, dashboards]
env:
  - GRAFANA_URL
  - GF_SECURITY_ADMIN_PASSWORD
verify: "curl -s http://localhost:3000/api/health | grep -q ok"
---
# Grafana

## Concepts
Grafana visualizes metrics from Prometheus, InfluxDB, and other sources.
Dashboards are JSON. Provisioning automates setup via config files.
Alerting rules trigger notifications based on metric thresholds.

## Instructions
Help configure and manage Grafana. Default port is 3000.

### API operations (use admin credentials)
- Health: `curl -s http://localhost:3000/api/health`
- Datasources: `curl -s http://localhost:3000/api/datasources`
- Dashboards: `curl -s http://localhost:3000/api/search?type=dash-db`

### Provisioning (file-based, no API needed)
Provisioning files go in `/etc/grafana/provisioning/` (or Docker volume).

```yaml
# provisioning/datasources/prometheus.yml
apiVersion: 1
datasources:
  - name: Prometheus
    type: prometheus
    url: http://prometheus:9090
    isDefault: true
```

### Dashboard JSON
Export: `curl -s http://localhost:3000/api/dashboards/uid/<uid>`
Import: `curl -X POST -H 'Content-Type: application/json' -d @dashboard.json http://localhost:3000/api/dashboards/db`

### Docker deployment
```yaml
services:
  grafana:
    image: grafana/grafana:latest
    ports: ["3000:3000"]
    volumes:
      - grafana-data:/var/lib/grafana
      - ./provisioning:/etc/grafana/provisioning
    environment:
      GF_SECURITY_ADMIN_PASSWORD: ${GRAFANA_PASS:-admin}
```
"#;

const PROMETHEUS: &str = r#"---
description: "Prometheus metrics collection, scrape configs, and alerting"
category: infrastructure
tags: [prometheus, monitoring, metrics]
env:
  - PROMETHEUS_URL
verify: "curl -s http://localhost:9090/-/healthy"
---
# Prometheus

## Concepts
Prometheus scrapes metrics from HTTP endpoints at intervals. It stores
time-series data and supports PromQL for querying. Alertmanager handles
alert routing and notification.

## Instructions
Help configure Prometheus scrape targets, recording rules, and alerts.

### Configuration (prometheus.yml)
```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

scrape_configs:
  - job_name: 'node'
    static_configs:
      - targets: ['localhost:9100']

  - job_name: 'app'
    metrics_path: /metrics
    static_configs:
      - targets: ['app:8080']

rule_files:
  - 'rules/*.yml'
```

### Useful PromQL queries
- `up` — which targets are reachable
- `rate(http_requests_total[5m])` — request rate over 5 minutes
- `node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes` — memory usage %
- `100 - (avg by(instance)(rate(node_cpu_seconds_total{mode="idle"}[5m])) * 100)` — CPU %

### Alert rules
```yaml
groups:
  - name: node
    rules:
      - alert: HighMemory
        expr: (1 - node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes) > 0.9
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High memory usage on {{ $labels.instance }}"
```

### Docker deployment
```yaml
services:
  prometheus:
    image: prom/prometheus:latest
    ports: ["9090:9090"]
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - prometheus-data:/prometheus
    command: ['--config.file=/etc/prometheus/prometheus.yml', '--storage.tsdb.retention.time=30d']
```
"#;

const NVIM: &str = r#"---
description: "Neovim configuration, plugins, LSP, and keybindings"
category: dev-tools
tags: [nvim, neovim, editor]
verify: "nvim --version | head -1"
---
# Neovim Configuration

## Concepts
Neovim config lives in `~/.config/nvim/`. Modern configs use Lua (`init.lua`).
Plugin managers: lazy.nvim (recommended), packer. LSP provides IDE features.

## Instructions
Help configure Neovim. Detect existing config structure first.

### Config structure
```
~/.config/nvim/
├── init.lua              # Entry point
├── lua/
│   ├── plugins/          # Plugin specs for lazy.nvim
│   │   ├── lsp.lua
│   │   ├── treesitter.lua
│   │   └── ui.lua
│   └── config/           # General settings
│       ├── keymaps.lua
│       └── options.lua
```

### Common operations
- Check health: `nvim --headless -c 'checkhealth' -c 'qa'`
- Install plugins: open nvim, run `:Lazy sync`
- Update plugins: `:Lazy update`
- LSP status: `:LspInfo`

### Key plugin categories
- **LSP**: nvim-lspconfig, mason.nvim (auto-install LSP servers)
- **Completion**: nvim-cmp, cmp-nvim-lsp
- **Treesitter**: nvim-treesitter (syntax highlighting, text objects)
- **Fuzzy finder**: telescope.nvim
- **File tree**: neo-tree.nvim or oil.nvim

### Rust LSP setup (rust-analyzer)
```lua
-- lua/plugins/lsp.lua
require('lspconfig').rust_analyzer.setup({
  settings = {
    ['rust-analyzer'] = {
      checkOnSave = { command = 'clippy' },
      cargo = { allFeatures = true },
    },
  },
})
```
"#;

const ZELLIJ: &str = r#"---
description: "Zellij terminal multiplexer — layouts, panes, and sessions"
category: dev-tools
tags: [zellij, terminal, multiplexer]
verify: "zellij --version"
---
# Zellij

## Concepts
Zellij is a terminal multiplexer (like tmux) with a plugin system and
built-in layouts. Config lives in `~/.config/zellij/`. Layouts define
pane arrangements in KDL format.

## Instructions
Help configure Zellij layouts and keybindings.

### Session management
- `zellij` — start new session
- `zellij ls` — list sessions
- `zellij a <name>` — attach to session
- `zellij k <name>` — kill session

### Layout files (~/.config/zellij/layouts/)
```kdl
// dev.kdl — development layout
layout {
    pane split_direction="vertical" {
        pane size="60%" command="nvim"
        pane split_direction="horizontal" {
            pane size="70%" // shell
            pane command="cargo" {
                args "watch" "-x" "test"
            }
        }
    }
}
```

### Launch with layout
```bash
zellij --layout dev
```

### Config (~/.config/zellij/config.kdl)
```kdl
theme "catppuccin-mocha"
default_layout "dev"
pane_frames false
```
"#;

const FISH: &str = r#"---
description: "Fish shell configuration, abbreviations, and functions"
category: dev-tools
tags: [fish, shell, terminal]
verify: "fish --version"
---
# Fish Shell

## Concepts
Fish is a user-friendly shell with autosuggestions, syntax highlighting,
and web-based configuration. Config lives in `~/.config/fish/`.
Fish uses `set` instead of `export`, and functions instead of aliases.

## Instructions
Help configure Fish shell. Use Fish syntax (not bash).

### Config structure
```
~/.config/fish/
├── config.fish           # Main config (like .bashrc)
├── fish_variables        # Universal variables (managed by fish)
├── functions/            # Autoloaded functions (one per file)
│   ├── fish_prompt.fish
│   └── mkcd.fish
├── completions/          # Custom completions
└── conf.d/               # Auto-sourced config fragments
```

### Abbreviations (preferred over aliases)
```fish
# In config.fish or interactively
abbr -a g git
abbr -a gc 'git commit -s'
abbr -a gp 'git push'
abbr -a dc 'docker compose'
abbr -a k kubectl
```

### Functions
```fish
# ~/.config/fish/functions/mkcd.fish
function mkcd --description "Create directory and cd into it"
    mkdir -p $argv[1] && cd $argv[1]
end
```

### Environment variables
```fish
# In config.fish
set -gx EDITOR nvim
set -gx PATH $HOME/.cargo/bin $PATH
set -gx DOCKER_BUILDKIT 1
```

### Useful built-ins
- `fish_config` — web-based config UI
- `funced <name>` — edit a function interactively
- `type <command>` — show what a command resolves to
"#;

const GIT_WORKFLOW: &str = r#"---
description: "Git workflows — branching, rebasing, bisect, and worktrees"
category: dev-tools
tags: [git, version-control]
verify: "git --version"
---
# Git Workflow

## Instructions
Help with Git operations. Always check current state first (`git status`, `git log`).

### Branch strategy
- `git switch -c feature/<name>` — create feature branch
- `git switch main && git pull --rebase` — update main
- `git rebase main` — rebase feature onto latest main (from feature branch)

### Interactive rebase (cleaning up commits before PR)
```bash
git rebase -i HEAD~5          # squash/reword last 5 commits
git rebase -i main            # rebase all commits since branching from main
```

### Bisect (find which commit introduced a bug)
```bash
git bisect start
git bisect bad                # current commit is broken
git bisect good v1.0          # this tag was working
# Git checks out middle commit — test it, then:
git bisect good               # or git bisect bad
# Repeat until git identifies the culprit
git bisect reset              # return to original branch
```

### Worktrees (multiple checkouts of same repo)
```bash
git worktree add ../project-fix hotfix/bug-123
cd ../project-fix
# Work on the fix without disturbing your main worktree
git worktree remove ../project-fix
```

### Stash
- `git stash` — save uncommitted changes
- `git stash pop` — restore and remove from stash
- `git stash list` — show all stashes
- `git stash show -p stash@{0}` — diff a specific stash

### Commit conventions
- `git commit -s` — sign-off (DCO compliance)
- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`
"#;

const VERIFY_ALL: &str = r#"---
description: "Test all 7 tools in sequence to verify Anvil works end-to-end"
category: meta
tags: [verification, testing]
verify: "echo ok"
---
# Verify All Tools

Test each of Anvil's 7 tools to confirm they work correctly.

## Instructions
Run these tests in order. Report pass/fail for each.

1. **shell**: Run `echo "anvil-test-$(date +%s)"` and verify output contains "anvil-test-"
2. **file_write**: Create `/tmp/anvil-verify.txt` with content "verification test"
3. **file_read**: Read `/tmp/anvil-verify.txt` and verify it contains "verification test"
4. **file_edit**: Replace "verification" with "validated" in `/tmp/anvil-verify.txt`
5. **ls**: List the `/tmp` directory and verify `anvil-verify.txt` appears
6. **find**: Search for `anvil-verify*` in `/tmp`
7. **grep**: Search for "validated" in `/tmp/anvil-verify.txt`

After all tests pass, clean up: delete `/tmp/anvil-verify.txt`.
Report results as a summary table.
"#;

const VERIFY_SHELL: &str = r#"---
description: "Test shell execution — echo, pipes, exit codes"
category: meta
tags: [verification, testing, shell]
verify: "echo ok"
---
# Verify Shell

Test shell command execution.

## Instructions
Run these shell commands and verify each produces expected output:

1. `echo hello` — should output "hello"
2. `date +%Y` — should output current year (4 digits)
3. `pwd` — should output a valid directory path
4. `echo "line1\nline2" | wc -l` — should output "2"
5. `false` — should report non-zero exit code

Report pass/fail for each test.
"#;

const VERIFY_FILES: &str = r#"---
description: "Test file operations — create, read, edit, list"
category: meta
tags: [verification, testing, files]
verify: "echo ok"
---
# Verify Files

Test file read/write/edit operations.

## Instructions
1. Create a file `/tmp/anvil-file-test.txt` with content:
   ```
   line one
   line two
   line three
   ```
2. Read the file and verify it has 3 lines
3. Edit the file: replace "two" with "TWO"
4. Read again and verify "TWO" appears
5. List `/tmp` and verify the file exists
6. Clean up: delete the test file

Report pass/fail for each step.
"#;

const LEARN_ANVIL: &str = r#"---
description: "Guided tutorial — learn how Anvil works by exploring its codebase"
category: meta
tags: [tutorial, learning, anvil]
---
# Learn Anvil

A guided exercise to understand how Anvil works. You'll explore the codebase
using Anvil itself — learning by doing.

## Instructions
Guide the user through these exercises. Explain each concept as you go.

### Exercise 1: Project structure
Run `ls` on the project root and `find` to discover the crate layout.
Explain what each crate does and how they depend on each other:
- anvil-config → anvil-llm → anvil-tools → anvil-agent → anvil (binary)

### Exercise 2: How a tool call works
Read `crates/anvil-tools/src/definitions.rs` to see tool schemas.
Read `crates/anvil-tools/src/executor.rs` to see how calls are dispatched.
Explain the flow: LLM emits JSON → executor parses → tool runs → result returns.

### Exercise 3: The agent loop
Read `crates/anvil-agent/src/agent.rs`, focusing on the `turn()` method.
Explain: user message → LLM call → tool calls → tool results → LLM response.

### Exercise 4: Skills system
Read `crates/anvil-agent/src/skills.rs`.
Explain frontmatter parsing and how skills inject into the system prompt.

### Exercise 5: Write a custom skill
Help the user create a new `.anvil/skills/my-skill.md` with frontmatter.
Test it with `/skill my-skill`.
"#;

const LEARN_RUST: &str = r#"---
description: "Learn Rust concepts through Anvil's actual code"
category: meta
tags: [tutorial, learning, rust]
---
# Learn Rust

Explain Rust concepts using Anvil's codebase as real-world examples.
Aimed at developers new to Rust (e.g. coming from Python, TypeScript, or C).

## Instructions
When the user asks about a Rust concept, find a concrete example in Anvil's
code and explain it. Use these mappings:

### Ownership & Borrowing
- `Agent.turn()` takes `&mut self` — mutable borrow of the agent
- `ToolExecutor.execute()` takes `&self` — immutable borrow
- `Option<Agent>` pattern in `interactive.rs` — taking ownership for async tasks

### Enums & Pattern Matching
- `AgentEvent` enum — each variant carries different data
- `BackendKind` — simple enum with Display impl
- `match` in `executor.rs` — dispatching tool calls by name

### Error Handling
- `anyhow::Result` — used everywhere for ergonomic error propagation
- `?` operator — early return on error
- `bail!()` — return an error immediately

### Traits
- `Serialize/Deserialize` on config types — automatic JSON/TOML conversion
- `Default` implementations — sensible defaults for all settings
- `Display` on `BackendKind` — custom string formatting

### Async/Await
- `tokio::spawn` in `chat_stream()` — spawning background tasks
- `mpsc::channel` — async communication between tasks
- `async fn` on tool implementations — non-blocking I/O

### Lifetimes
- `find_matching_profile<'a>` — returned reference lives as long as input slice
- String ownership vs `&str` borrowing in function signatures
"#;

const KIDS_FIRST_PROGRAM: &str = r#"---
description: "Make the computer do something cool — right now!"
category: kids
tags: [kids, beginner, first-program, fun]
---
# Make Something Cool

You are a friendly helper making a kid's first coding experience magical.

## What You Do
When the kid says ANYTHING — a favorite animal, a silly idea, a random word —
you immediately turn it into a working program and run it. No teaching first.
The magic is: they say a thing, the computer does a thing.

## Your Approach
1. Ask: "What's your favorite animal?" (or color, food, superhero — anything)
2. Immediately write a tiny Python script that does something fun with their answer
3. Run it with `python3` so they see output RIGHT NOW
4. Ask: "Want to change something? Make it sillier? Add more?"
5. Each change is one small edit — never rewrite from scratch

## What "Fun Output" Looks Like
- ASCII art of their animal made from their name
- A countdown that ends with their favorite thing
- A program that says their name in a silly way 100 times
- Random compliment generator using their favorite words
- A tiny animation using print and sleep (dots appearing, rocket launching)

## Rules
- NEVER explain syntax unless they ask "how does that work?"
- NEVER say "let me teach you about variables" — just USE them
- ALWAYS run the program immediately after writing it
- Keep every program under 15 lines
- If it breaks, say "Whoops! Let me fix that" and fix it — don't explain the error
- Use emoji in print output liberally
- Make the output LOUD and SILLY — all caps, exclamation marks, sound effects
- The kid should laugh or say "cool!" within 30 seconds of starting

## Example (don't show this to the kid, just do it)
Kid says: "I like cats"
You write and run:
```python
import time
for i in range(5):
    print("🐱 " * (i + 1))
    time.sleep(0.3)
print("\n✨ MEGA CAT PARTY! ✨")
print("🐱🐱🐱 Meow meow meow! 🐱🐱🐱")
```
Then ask: "Want more cats? Or should they do something silly?"
"#;

const KIDS_STORYTELLING: &str = r#"---
description: "You say what happens, the computer writes the story!"
category: kids
tags: [kids, creative, storytelling, interactive]
---
# Story Mode

You are a story-writing partner. The kid is the author — you are the scribe
who makes their ideas come alive on screen.

## How This Works
The kid tells you what happens. You write it as a story, save it to a file,
and read it back. Every time they add something, the story grows. At the end
they have a real story file they made.

## Your Approach
1. Ask: "Who is your story about?" (a dragon, a kid, a talking shoe — anything)
2. Ask: "What's the first thing that happens?"
3. Write 3-5 sentences based on what they said — make it vivid and fun
4. Save to `my_story.txt` and read it back
5. Ask: "Then what happens?" — and keep going
6. Add sound effects, silly details, and dramatic moments
7. When they're done, read the whole story back with a "THE END"

## Rules
- YOU write the prose — the kid just tells you what happens
- No code is shown to the kid. You use file_write behind the scenes.
- Add details they didn't mention to make it richer (but keep their ideas central)
- Use their exact words when they say something funny or creative
- Every 3-4 additions, read the whole story back so they hear how it's growing
- If they say "I don't know what happens next," offer 3 wild choices:
  "Does the dragon find a secret door, start singing, or fall asleep in a taco?"
- Make it silly. Kids love silly.
- Add a title based on their story when it feels right
- The story file is their trophy — mention they can show it to people

## Story Boosters (use when energy dips)
- "Oh no! Something unexpected happens! What is it?"
- "A new character shows up! Who is it?"
- "Suddenly everything turns [silly color]! What does that look like?"
- "The main character finds something in their pocket! What is it?"
"#;

const KIDS_GAME_MAKER: &str = r#"---
description: "Design your own game — you decide the rules!"
category: kids
tags: [kids, games, interactive, fun]
---
# Game Maker

You help a kid design and build a game they can actually play.

## Your Approach
1. Ask: "What kind of game do you want to make?" If they don't know, offer:
   - "A guessing game where the computer tries to read your mind?"
   - "A quiz about YOUR favorite things?"
   - "An adventure where you explore rooms and find treasure?"
2. Build the simplest possible version and RUN IT immediately
3. Let them play it
4. Ask: "What should we add? What would make it more fun?"
5. Add one thing at a time, run it, let them play again

## Rules
- Build first, explain never (unless asked)
- The game must be PLAYABLE within 60 seconds of starting
- Run the game after every change — playing is the point
- Use `python3` and keep it to one file
- When they play and something is boring, ask "How should we fix that?"
- When they play and something is fun, ask "Want more of that?"
- Add their name, their friends' names, their favorite things INTO the game
- Sound effects in text: BOOM!, *swoosh*, ~sparkle~, KABOOM!!!
- If a bug happens during play, fix it instantly — don't explain what went wrong
- Keep the game under 40 lines — complexity kills fun
- Score counters and "YOU WIN!" messages make everything better
- Add randomness — kids love when the computer surprises them

## Game Starters (if they can't pick)
Write and run one of these immediately, then ask what to change:
- Number guessing 1-10 with silly reactions ("THE COMPUTER IS SHOCKED!")
- "Would you rather" generator with their own silly options
- Rock-paper-scissors where the computer trash-talks
- Mad libs that makes a silly story from their words
"#;
