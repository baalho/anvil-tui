use serde_json::{json, Value};

pub fn all_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "file_read",
                "description": "Read the contents of a file. Optionally specify line range.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to read"
                        },
                        "start_line": {
                            "type": "integer",
                            "description": "First line to read (1-based, inclusive)"
                        },
                        "end_line": {
                            "type": "integer",
                            "description": "Last line to read (1-based, inclusive)"
                        }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "file_write",
                "description": "Write content to a file. Creates the file if it doesn't exist, overwrites if it does.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to write"
                        },
                        "content": {
                            "type": "string",
                            "description": "Content to write to the file"
                        }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "file_edit",
                "description": "Edit a file by replacing an exact string match with new content. The old_str must match exactly one location in the file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to edit"
                        },
                        "old_str": {
                            "type": "string",
                            "description": "Exact string to find and replace (must be unique in the file)"
                        },
                        "new_str": {
                            "type": "string",
                            "description": "Replacement string. Omit to delete old_str."
                        }
                    },
                    "required": ["path", "old_str"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Execute a shell command. Returns stdout, stderr, and exit code.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute, e.g. \"ls -la src/\""
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds (optional)"
                        },
                        "working_dir": {
                            "type": "string",
                            "description": "Working directory for the command (optional)"
                        }
                    },
                    "required": ["command"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search file contents using a regex pattern.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex pattern to search for"
                        },
                        "path": {
                            "type": "string",
                            "description": "File or directory to search in"
                        },
                        "include": {
                            "type": "string",
                            "description": "Glob pattern for files to include (e.g. \"*.rs\")"
                        }
                    },
                    "required": ["pattern", "path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "ls",
                "description": "List directory contents with file types and sizes. Skips .git, node_modules, target by default.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory to list (default: current directory)"
                        },
                        "all": {
                            "type": "boolean",
                            "description": "Include hidden files (default: false)"
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "find",
                "description": "Find files matching a glob pattern recursively. Skips .git, node_modules, target.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern to match, e.g. \"*.rs\" or \"Cargo.toml\""
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory to search in (default: current directory)"
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Maximum directory depth (default: 10)"
                        }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "Show the working tree status. Returns modified, staged, and untracked files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "verbose": {
                            "type": "boolean",
                            "description": "Show full status instead of short format (default: false)"
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_diff",
                "description": "Show changes between commits, staging area, and working tree.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "staged": {
                            "type": "boolean",
                            "description": "Show staged (cached) changes instead of unstaged (default: false)"
                        },
                        "ref": {
                            "type": "string",
                            "description": "Git ref to diff against (e.g. HEAD~1, main, a commit SHA)"
                        },
                        "path": {
                            "type": "string",
                            "description": "Limit diff to a specific file or directory"
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_log",
                "description": "Show recent commit history.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "count": {
                            "type": "integer",
                            "description": "Number of commits to show (default: 10)"
                        },
                        "oneline": {
                            "type": "boolean",
                            "description": "One-line format (default: true)"
                        },
                        "path": {
                            "type": "string",
                            "description": "Show commits affecting this file or directory"
                        }
                    },
                    "required": []
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "git_commit",
                "description": "Stage files and create a git commit.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "Commit message"
                        },
                        "files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Files to stage before committing. If empty and all=true, stages all modified files."
                        },
                        "all": {
                            "type": "boolean",
                            "description": "Stage all modified tracked files (-a flag, default: false)"
                        }
                    },
                    "required": ["message"]
                }
            }
        }),
    ]
}
