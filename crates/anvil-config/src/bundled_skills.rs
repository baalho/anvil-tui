//! Bundled skill content shipped with `anvil init`.
//!
//! Each skill is a (filename, content) pair. Skills use YAML frontmatter
//! for metadata and markdown for the prompt template. They serve dual purpose:
//! 1. Prompt template — injected into the system prompt when activated
//! 2. Documentation — teaches concepts through examples and explanations

/// All bundled skills, written to `.anvil/skills/` by `init_harness()`.
pub const BUNDLED_SKILLS: &[(&str, &str)] = &[
    // --- Infrastructure ---
    ("containers.md", CONTAINERS),
    ("server-admin.md", SERVER_ADMIN),
    ("sops-age.md", SOPS_AGE),
    ("deploy.md", DEPLOY),
    ("deploy-fish.md", DEPLOY_FISH),
    ("tailscale.md", TAILSCALE),
    ("caddy-cloudflare.md", CADDY_CLOUDFLARE),
    ("restic-backup.md", RESTIC_BACKUP),
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

const CONTAINERS: &str = r#"---
description: "Manage containers and compose stacks — Docker or Podman"
category: infrastructure
tags: [docker, podman, containers, compose]
env:
  - DOCKER_HOST
  - CONTAINER_HOST
  - COMPOSE_FILE
  - COMPOSE_PROJECT_NAME
verify: "command -v podman || command -v docker"
---
# Container Management

## Runtime Detection
Detect which runtime is available before running commands:
```bash
if command -v podman &>/dev/null; then
  RUNTIME=podman
  COMPOSE="podman-compose"
elif command -v docker &>/dev/null; then
  RUNTIME=docker
  COMPOSE="docker compose"
fi
```
Docker and Podman CLIs are compatible — most commands work with either.

## Instructions
Detect the runtime first, then use the appropriate commands.

### Container lifecycle
- `$RUNTIME ps -a` — list all containers (running and stopped)
- `$RUNTIME run -d --name <n> <image>` — start detached container
- `$RUNTIME logs -f --tail 100 <n>` — follow logs, last 100 lines
- `$RUNTIME exec -it <n> sh` — interactive shell in running container
- `$RUNTIME stop <n> && $RUNTIME rm <n>` — clean shutdown and removal

### Compose lifecycle
Docker uses `docker compose` (v2 plugin). Podman uses `podman-compose`.
- `$COMPOSE up -d` — start all services detached
- `$COMPOSE down` — stop and remove containers (keeps volumes)
- `$COMPOSE down -v` — stop and remove containers AND volumes
- `$COMPOSE restart <service>` — restart one service
- `$COMPOSE pull` — pull latest images for all services
- `$COMPOSE ps` — service status
- `$COMPOSE logs -f <service>` — follow one service's logs

### Image management
- `$RUNTIME images` — list local images with sizes
- `$RUNTIME pull <image>:<tag>` — pull specific version
- `$RUNTIME build -t <name>:<tag> .` — build from Dockerfile/Containerfile
- `$RUNTIME image prune -f` — remove dangling images

### Volume and network
- `$RUNTIME volume ls` — list volumes
- `$RUNTIME volume create <name>` — create named volume
- `$RUNTIME network ls` — list networks

### Podman-specific
- Podman runs rootless by default — no `sudo` needed
- `podman generate systemd --new --name <container>` — create systemd unit
- `systemctl --user enable --now container-<name>.service` — auto-start on boot
- `podman pod create --name <pod> -p 8080:80` — create a pod (groups containers)
- `loginctl enable-linger <user>` — allow user services to run without login

### Debugging
- `$RUNTIME inspect <container>` — full container metadata as JSON
- `$RUNTIME stats --no-stream` — one-shot resource usage
- `$RUNTIME system df` — disk usage breakdown

## Examples
```yaml
# compose.yaml (works with docker compose and podman-compose)
services:
  app:
    image: myapp:latest
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
description: "System administration — services, disks, processes, SSH, Tailscale"
category: infrastructure
tags: [linux, macos, sysadmin, ssh, tailscale, podman]
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
If `.anvil/inventory.toml` exists, use it to look up hosts and services.

### Service management (Linux)
- `systemctl status <service>` — check service state
- `systemctl restart <service>` — restart a service
- `journalctl -u <service> -f --since '5 min ago'` — recent logs

### Podman user services (Linux)
Podman containers managed via systemd user units:
- `systemctl --user status <service>` — check rootless container service
- `systemctl --user restart <service>` — restart rootless container
- `journalctl --user -u <service> --since '5 min ago'` — container logs via journal
- `loginctl enable-linger <user>` — allow services to run without active login

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

### SSH over Tailscale
When hosts are on a Tailscale mesh, use MagicDNS hostnames:
- `ssh user@<tailscale-hostname>` — connect via Tailscale
- `ssh user@<tailscale-hostname> '<command>'` — run remote command
- `ssh user@<tailscale-hostname> 'cd /path && fish deploy.fish'` — remote deploy

### SSH (general)
- `ssh -T git@github.com` — test GitHub SSH
- `ssh-add -l` — list loaded SSH keys
- `ssh -L 8080:localhost:80 user@host` — port forward

## Examples
```bash
# Find what's eating disk space
du -sh /var/log/* | sort -rh | head -10

# Check if a port is in use
lsof -i :8080

# Check a remote Podman container via Tailscale
ssh deploy@debian-server 'podman ps'
```
"#;

const SOPS_AGE: &str = r#"---
description: "Encrypt and decrypt secrets with SOPS and Age"
category: infrastructure
tags: [sops, age, secrets, encryption, gitops]
env:
  - SOPS_AGE_KEY_FILE
  - SOPS_AGE_RECIPIENTS
verify: "sops --version && age --version"
---
# SOPS + Age Secrets Management

## Concepts
Age is a simple file encryption tool. SOPS (Secrets OPerationS) encrypts
specific values in structured files (YAML, JSON, ENV) using Age keys.
Together they enable GitOps: encrypted secrets committed to git, decrypted
only at deploy time, plaintext deleted immediately after use.

## Instructions
Help manage encrypted secrets. Never commit plaintext `.env` files.

### Age key management
- `age-keygen -o key.txt` — generate a new Age keypair
- The public key (starts with `age1...`) goes in `.sops.yaml`
- The private key file is set via `SOPS_AGE_KEY_FILE` env var
- Each host has its own Age keypair — distribute public keys, never private

### .sops.yaml configuration
```yaml
creation_rules:
  - path_regex: \.enc\.env$
    age: >-
      age1macbook...,
      age1debian1...,
      age1debian2...
```
This tells SOPS which Age recipients can decrypt files matching the path regex.

### Encrypting secrets
```bash
# Encrypt a .env file (output to .enc.env)
sops -e --input-type dotenv --output-type dotenv .env > .enc.env

# Encrypt in-place
sops -e -i secrets.yaml

# Encrypt with explicit recipient
sops -e --age age1... .env > .enc.env
```

### Decrypting secrets
```bash
# Decrypt to stdout
sops -d .enc.env

# Decrypt to file (for deploy scripts)
sops -d .enc.env > .env

# Always delete plaintext after use
rm .env
```

### Adding a new host
1. Generate Age keypair on the new host: `age-keygen -o ~/.config/sops/age/keys.txt`
2. Copy the public key (`age1...`)
3. Add it to `.sops.yaml` creation rules
4. Re-encrypt all secrets: `sops updatekeys .enc.env`

### GitOps pattern
- `.enc.env` → committed to git (encrypted, safe)
- `.env` → NEVER committed (in `.gitignore`)
- Deploy: `sops -d .enc.env > .env && compose up && rm .env`
"#;

const DEPLOY: &str = r#"---
description: "Deploy services to inventory hosts using SOPS/age secrets"
category: infrastructure
tags: [deploy, sops, age, inventory, homelab]
env:
  - SOPS_AGE_KEY_FILE
  - SSH_AUTH_SOCK
depends:
  - sops-age
---
# Service Deployment

Deploy containerized services to inventory hosts using SOPS/age for secrets
and SSH for remote execution.

## Prerequisites

- Inventory configured in `.anvil/inventory.toml` with host details
- SOPS/age encryption set up (see sops-age skill)
- SSH access to target hosts via Tailscale

## Deployment Workflow

1. **Verify target**: Check inventory for host and service
2. **Decrypt secrets**: `sops -d secrets/<service>.env > /tmp/<service>.env`
3. **Copy secrets**: `scp /tmp/<service>.env <user>@<host>:/opt/<service>/.env`
4. **Deploy**: `ssh <user>@<host> 'cd /opt/<service> && <runtime> compose pull && <runtime> compose up -d'`
5. **Verify**: `ssh <user>@<host> '<runtime> ps --filter name=<service>'`
6. **Cleanup**: `rm /tmp/<service>.env`

## Runtime Detection

Use the host's `container_runtime` from inventory:
- Docker hosts: `docker compose up -d`
- Podman hosts: `podman-compose up -d` or `podman compose up -d`

## Rollback

If deployment fails:
```bash
ssh <user>@<host> 'cd /opt/<service> && <runtime> compose down && <runtime> compose up -d --no-build'
```

## Security Notes

- NEVER leave decrypted secrets on disk — always clean up temp files
- Use `sops -d` to decrypt, pipe directly when possible
- Verify `SSH_AUTH_SOCK` is set for agent forwarding
- All secrets files should be in `.gitignore`
"#;

const DEPLOY_FISH: &str = r#"---
description: "Scaffold Fish shell deploy scripts — git pull, decrypt, compose up, cleanup"
category: infrastructure
tags: [fish, deploy, gitops, sops, compose]
env:
  - SOPS_AGE_KEY_FILE
  - DEPLOY_TARGET
depends:
  - sops-age
  - containers
verify: "fish --version && sops --version"
---
# Deploy Script (Fish Shell)

## Concepts
The deploy.fish pattern is a four-step GitOps deployment:
1. `git pull` — fetch latest encrypted configs from the repo
2. `sops -d .enc.env > .env` — decrypt secrets
3. `podman-compose up -d` or `docker compose up -d` — start services
4. `rm .env` — clean up plaintext secrets

Each service lives in its own git repo with a `compose.yaml` and `.enc.env`.

## Instructions
Help scaffold new deploy.fish scripts and run existing ones.

### Canonical deploy.fish template
```fish
#!/usr/bin/env fish
# Deploy script — git pull, decrypt, compose up, cleanup

set -l service_dir (dirname (status filename))
cd $service_dir

echo "Pulling latest config..."
git pull --ff-only
or begin
    echo "Git pull failed — resolve conflicts first"
    exit 1
end

echo "Decrypting secrets..."
sops -d --input-type dotenv --output-type dotenv .enc.env > .env
or begin
    echo "Decryption failed — check SOPS_AGE_KEY_FILE"
    exit 1
end

# Detect container runtime
if command -v podman-compose &>/dev/null
    set compose_cmd podman-compose
else if command -v docker &>/dev/null
    set compose_cmd docker compose
else
    echo "No container runtime found"
    rm -f .env
    exit 1
end

echo "Starting services with $compose_cmd..."
$compose_cmd up -d
set -l compose_status $status

# Always clean up plaintext secrets
rm -f .env

if test $compose_status -ne 0
    echo "Compose failed with status $compose_status"
    exit 1
end

echo "Deploy complete — verifying..."
$compose_cmd ps
```

### Remote deployment
```bash
# Run deploy.fish on a remote host via SSH
ssh user@<tailscale-host> 'cd /srv/valheim && fish deploy.fish'
```

### Rollback
```fish
# Revert to previous version
git stash
fish deploy.fish
```

### Scaffolding a new service
1. Create a git repo for the service
2. Add `compose.yaml` with service definition
3. Create `.env` with secrets, encrypt: `sops -e .env > .enc.env && rm .env`
4. Add `.env` to `.gitignore`
5. Copy the deploy.fish template above
6. Commit `.enc.env`, `compose.yaml`, `deploy.fish`, `.gitignore`
"#;

const TAILSCALE: &str = r#"---
description: "Manage Tailscale mesh VPN — status, connectivity, MagicDNS"
category: infrastructure
tags: [tailscale, vpn, mesh, networking]
env:
  - TS_AUTHKEY
verify: "tailscale version"
---
# Tailscale Mesh VPN

## Concepts
Tailscale creates a WireGuard-based mesh VPN (tailnet) between devices.
MagicDNS assigns hostnames to each node so you can `ssh server-name`
instead of remembering IPs. All traffic is encrypted end-to-end.

## Instructions
Help manage Tailscale connectivity and troubleshoot mesh issues.

### Node status
- `tailscale status` — list all nodes, their IPs, and online status
- `tailscale status --json` — machine-readable output
- `tailscale ip -4 <hostname>` — get a node's Tailscale IPv4 address
- `tailscale ping <hostname>` — test direct connectivity (vs relayed)

### Joining a node
```bash
# Interactive login
tailscale up

# Non-interactive with auth key (for servers)
tailscale up --authkey=tskey-auth-...

# Advertise as subnet router
tailscale up --advertise-routes=192.168.1.0/24

# Advertise as exit node
tailscale up --advertise-exit-node
```

### SSH over Tailscale
Tailscale MagicDNS lets you SSH by hostname:
```bash
ssh user@debian-server          # MagicDNS hostname
ssh user@100.64.0.2             # Tailscale IP (if MagicDNS is off)
```

### DNS and networking
- MagicDNS: enabled by default, resolves `<hostname>` within the tailnet
- `tailscale dns status` — show DNS configuration
- `tailscale netcheck` — diagnose connectivity (DERP relays, NAT type)

### Administration
- ACLs are managed in the Tailscale admin console (https://login.tailscale.com/admin/acls)
- `tailscale up --reset` — re-authenticate and reset node state
- `tailscale down` — disconnect from tailnet (keeps config)
- `tailscale logout` — fully deauthenticate

### Troubleshooting
- `tailscale ping <host>` shows "via DERP" → nodes can't connect directly (NAT issue)
- `tailscale netcheck` — check NAT type, DERP relay latency
- `tailscale bugreport` — generate diagnostic bundle
"#;

const CADDY_CLOUDFLARE: &str = r#"---
description: "Reverse proxy with Caddy — Cloudflare DNS challenge for HTTPS"
category: infrastructure
tags: [caddy, cloudflare, https, reverse-proxy, dns]
env:
  - CLOUDFLARE_API_TOKEN
  - CF_ZONE_ID
verify: "caddy version"
---
# Caddy + Cloudflare DNS

## Concepts
Caddy is a web server with automatic HTTPS. For private networks (not
reachable from the internet), Caddy uses the Cloudflare DNS-01 ACME
challenge to obtain certificates — it proves domain ownership by creating
a DNS TXT record via the Cloudflare API instead of serving a challenge file.

## Instructions
Help configure Caddy as a reverse proxy with Cloudflare DNS challenge.

### Custom build with Cloudflare plugin
Caddy needs the Cloudflare DNS plugin compiled in:
```bash
# Install xcaddy
go install github.com/caddyserver/xcaddy/cmd/xcaddy@latest

# Build Caddy with Cloudflare DNS plugin
xcaddy build --with github.com/caddy-dns/cloudflare
```

### Caddyfile with DNS challenge
```
service.example.com {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }
    reverse_proxy localhost:8080
}

another.example.com {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }
    reverse_proxy localhost:3000
}
```

### Wildcard certificate
```
*.example.com {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }

    @immich host immich.example.com
    handle @immich {
        reverse_proxy localhost:2283
    }

    @paperless host paperless.example.com
    handle @paperless {
        reverse_proxy localhost:8000
    }
}
```

### Cloudflare API token
Create a token at https://dash.cloudflare.com/profile/api-tokens with:
- Zone / Zone / Read
- Zone / DNS / Edit
Scope it to the specific zone (domain).

### Running in a container
```yaml
services:
  caddy:
    image: custom-caddy:latest  # built with xcaddy + cloudflare plugin
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile
      - caddy_data:/data
      - caddy_config:/config
    environment:
      CLOUDFLARE_API_TOKEN: ${CLOUDFLARE_API_TOKEN}
    restart: unless-stopped
volumes:
  caddy_data:
  caddy_config:
```

### Tailscale integration
Caddy on the Tailscale network can proxy to services on any node:
```
service.example.com {
    tls {
        dns cloudflare {env.CLOUDFLARE_API_TOKEN}
    }
    reverse_proxy http://debian-server-2:8080  # Tailscale MagicDNS hostname
}
```
"#;

const RESTIC_BACKUP: &str = r#"---
description: "Encrypted, deduplicated backups with Restic"
category: infrastructure
tags: [restic, backup, disaster-recovery]
env:
  - RESTIC_REPOSITORY
  - RESTIC_PASSWORD
  - AWS_ACCESS_KEY_ID
  - AWS_SECRET_ACCESS_KEY
verify: "restic version"
---
# Restic Backups

## Concepts
Restic creates encrypted, deduplicated backups. Each backup is a snapshot.
Repositories can be local, SFTP, or S3-compatible. Deduplication means
only changed blocks are stored, making incremental backups fast and small.

## Instructions
Help manage Restic backup repositories, snapshots, and restore operations.

### Repository initialization
```bash
# Local repository
restic init -r /backup/repo

# SFTP repository (via Tailscale)
restic init -r sftp:user@backup-server:/backup/repo

# S3-compatible (MinIO, Backblaze B2, etc.)
restic init -r s3:https://s3.example.com/bucket-name
```

### Backup
```bash
# Backup a directory
restic backup /srv/data --tag myservice

# Backup with exclusions
restic backup /srv/data --exclude='*.log' --exclude='.cache'

# Backup multiple paths
restic backup /srv/service1 /srv/service2 --tag services
```

### Snapshots
```bash
# List all snapshots
restic snapshots

# List snapshots for a specific tag
restic snapshots --tag myservice

# Show files in a snapshot
restic ls latest
```

### Restore
```bash
# Restore latest snapshot to a target directory
restic restore latest --target /restore/path

# Restore a specific snapshot
restic restore abc123 --target /restore/path

# Restore specific files
restic restore latest --target /restore/path --include '/srv/data/config'
```

### Retention and pruning
```bash
# Apply retention policy and remove old data
restic forget --keep-daily 7 --keep-weekly 4 --keep-monthly 6 --prune

# Dry run first
restic forget --keep-daily 7 --keep-weekly 4 --keep-monthly 6 --dry-run
```

### Maintenance
```bash
# Verify repository integrity
restic check

# Full data verification (slow but thorough)
restic check --read-data
```

### Automated backups (systemd timer)
```ini
# /etc/systemd/system/restic-backup.service
[Unit]
Description=Restic backup

[Service]
Type=oneshot
EnvironmentFile=/etc/restic/env
ExecStart=restic backup /srv/data --tag automated
ExecStartPost=restic forget --keep-daily 7 --keep-weekly 4 --keep-monthly 6 --prune
```

```ini
# /etc/systemd/system/restic-backup.timer
[Unit]
Description=Daily Restic backup

[Timer]
OnCalendar=*-*-* 02:00:00
Persistent=true

[Install]
WantedBy=timers.target
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
