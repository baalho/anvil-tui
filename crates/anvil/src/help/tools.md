# Built-in Tools

Anvil provides 7 built-in tools that the LLM can call:

## file_read
Read file contents. Optionally specify a line range.
  Parameters: path (required), start_line, end_line

## file_write
Create or overwrite a file.
  Parameters: path (required), content (required)

## file_edit
Search-and-replace within a file. The old_str must match exactly once.
  Parameters: path (required), old_str (required), new_str (optional — omit to delete)

## shell
Execute a shell command. Returns stdout, stderr, and exit code.
  Parameters: command (required), timeout, working_dir

## grep
Search file contents using a regex pattern.
  Parameters: pattern (required), path (required), include (glob filter)

## ls
List directory contents with file types and sizes.
  Parameters: path, all (include hidden files)

## find
Find files matching a glob pattern recursively.
  Parameters: pattern (required), path, max_depth

## Custom Tools
Define custom tools in `.anvil/tools/*.toml`:

```toml
name = "deploy"
description = "Deploy to environment"

[[params]]
name = "environment"
type = "string"
required = true

[command]
template = "deploy.sh --env {{environment}}"
```
