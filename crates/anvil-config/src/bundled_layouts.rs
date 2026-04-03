//! Bundled Zellij layouts — shipped with `anvil init`.
//!
//! Anvil follows the BYOB (Bring Your Own Backend) model: Zellij manages
//! the inference server lifecycle, not Anvil. These layouts package the
//! correct llama-server flags and Anvil launch commands into ready-to-use
//! terminal workspaces.
//!
//! # Usage
//! ```bash
//! anvil --zellij anvil-tq    # TurboQuant layout
//! anvil --zellij anvil-dev   # Development layout
//! anvil --zellij anvil-ops   # Homelab operations layout
//! ```

/// Bundled Zellij layout files created by `anvil init`.
///
/// Each entry is `(filename, content)`. Written to `.anvil/layouts/`.
pub const BUNDLED_LAYOUTS: &[(&str, &str)] = &[
    (
        "anvil-tq.kdl",
        r#"// Anvil TurboQuant Layout
// Runs llama-server with TurboQuant KV cache in one pane, Anvil in another.
// Zellij manages both processes — closing the session kills everything cleanly.
//
// SETUP: Edit the variables below, then run:
//   zellij --layout .anvil/layouts/anvil-tq.kdl

layout {
    pane size="30%" borderless=true {
        // TurboQuant llama-server
        // Edit these paths and settings for your setup:
        //   MODEL: path to your GGUF model file
        //   CONTEXT: context window size (262144 for turbo4, 524288 for turbo3)
        //   CACHE_K/CACHE_V: KV cache quantization types
        command "sh"
        args "-c" "echo '=== TurboQuant llama-server ===' && echo 'Edit this pane command in .anvil/layouts/anvil-tq.kdl' && echo '' && echo 'Example:' && echo '  llama-server \\' && echo '    -m ~/models/qwen3-coder-30b-q4.gguf \\' && echo '    --cache-type-k q8_0 --cache-type-v turbo4 \\' && echo '    --jinja -ngl 99 -c 262144 -fa on \\' && echo '    --host 0.0.0.0 --port 8080' && echo '' && echo 'Replace this command with your llama-server launch line.' && exec sh"
    }
    pane size="70%" focus=true {
        // Anvil agent
        command "anvil"
    }
}
"#,
    ),
    (
        "anvil-dev.kdl",
        r#"// Anvil Development Layout
// Three-pane workspace: Anvil agent, editor, and shell.
//
// Usage:
//   zellij --layout .anvil/layouts/anvil-dev.kdl

layout {
    pane split_direction="vertical" {
        pane size="60%" focus=true {
            // Anvil agent
            command "anvil"
        }
        pane size="40%" split_direction="horizontal" {
            pane size="50%" {
                // Editor — open nvim, helix, or your preferred editor here
            }
            pane size="50%" {
                // Shell — run builds, tests, git commands
            }
        }
    }
}
"#,
    ),
    (
        "anvil-ops.kdl",
        r#"// Anvil Homelab Operations Layout
// Three-pane workspace for infrastructure management.
//
// Usage:
//   zellij --layout .anvil/layouts/anvil-ops.kdl

layout {
    pane split_direction="vertical" {
        pane size="50%" focus=true {
            // Anvil with homelab persona
            command "anvil"
            args "-p" "homelab"
        }
        pane size="50%" split_direction="horizontal" {
            pane size="50%" {
                // SSH / deployment shell
            }
            pane size="50%" {
                // Logs — tail -f, journalctl, docker logs, etc.
            }
        }
    }
}
"#,
    ),
];
