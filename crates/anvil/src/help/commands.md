# Slash Commands

Available commands in interactive mode:

## Session
  /help              — show this help
  /stats             — show session statistics (tokens, cost, routes)
  /compact           — summarize conversation to free context space
  /clear             — clear conversation history

## Model & Backend
  /model [name]      — show or switch the active model
  /backend [url]     — show or switch the backend
  /backend start     — start a managed llama-server
  /backend stop      — stop the managed backend
  /route <tool> <model> — route tool calls to a specific model

## Skills & Memory
  /skill [name]      — list or activate a skill
  /skill verify <n>  — verify a skill's prerequisites
  /memory add <text> — add a memory note
  /memory clear      — clear all memory notes

## Autonomous Mode
  /ralph --verify "cmd" [--max-iterations N]
                     — run autonomous loop until verification passes

## Display
  /think             — toggle thinking block visibility

## CLI Subcommands
  anvil init         — create .anvil/ harness directory
  anvil run -p "..." — run a single prompt non-interactively
  anvil history      — list past sessions
  anvil docs <topic> — show docs (tools, skills, config, commands)
