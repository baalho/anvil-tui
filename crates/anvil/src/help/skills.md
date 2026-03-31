# Skills System

Skills are markdown files in `.anvil/skills/` that provide domain-specific
instructions to the LLM. When activated, a skill's content is injected into
the system prompt.

## Activating Skills
  /skill <name>        — activate a skill
  /skill               — list available skills
  /skill verify <name> — run the skill's verification command

## Creating a Skill
Create a `.md` file in `.anvil/skills/`:

```markdown
---
description: "Manage Docker containers"
category: infrastructure
tags: [docker, containers]
env:
  - DOCKER_HOST
verify: "docker info"
depends: [git-workflow]
---
# Docker Management

Use docker commands to manage containers.
```

## Frontmatter Fields
- description — one-line description for listing
- category — organizational grouping
- tags — searchable tags
- env — environment variables to pass to shell commands
- verify — shell command to check prerequisites (exit 0 = pass)
- depends — other skills to auto-activate (transitive)

## Bundled Skills
Anvil ships with 14 bundled skills installed by `anvil init`.
