//! Agent mode — controls whether tools are available or the model responds directly.
//!
//! # Why modes exist
//! "Make a python file" should use `file_write`. "Print ASCII art" should respond
//! directly. Without modes, the model has no signal about which behavior is expected.
//! Modes set `tool_choice` in the API request and adjust the system prompt.
//!
//! # How it works
//! - `Coding` mode: tools available, `tool_choice: "auto"`, model decides when to use them
//! - `Creative` mode: tools omitted, `tool_choice: "none"`, model responds directly
//!
//! Personas set a default mode (kids personas → Creative, homelab → Coding).
//! The user can override with `/mode` at any time.

use std::fmt;

/// The agent's operating mode — determines tool availability and response style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Tools available, `tool_choice: "auto"`. Default for coding work.
    Coding,
    /// Tools omitted, model responds directly. For creative output, chat, stories.
    Creative,
}

impl Mode {
    /// The default mode when no persona is active.
    pub fn default_mode() -> Self {
        Self::Coding
    }

    /// The default mode for a given persona key.
    ///
    /// All personas default to Coding mode — tools are essential for every
    /// use case. Kids personas need `file_write` + `shell` to create and run
    /// programs. The persona provides personality; mode controls tool access.
    /// Users can override with `/mode creative` for pure chat.
    pub fn for_persona(_persona_key: &str) -> Self {
        Self::Coding
    }

    /// System prompt suffix injected when this mode is active.
    /// Returns `None` for Coding (no suffix needed — tools speak for themselves).
    pub fn prompt_suffix(&self) -> Option<&'static str> {
        match self {
            Self::Coding => None,
            Self::Creative => Some(
                "\n## Mode: Creative\n\
                 Respond directly with your output. Do not use tools unless the user \
                 explicitly asks you to create a file, run a command, or edit code. \
                 Focus on producing the requested content inline.\n",
            ),
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Self::default_mode()
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coding => write!(f, "coding"),
            Self::Creative => write!(f, "creative"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_coding() {
        assert_eq!(Mode::default(), Mode::Coding);
    }

    #[test]
    fn kids_personas_default_to_coding() {
        // Kids personas need tools (file_write + shell) to create and run programs
        assert_eq!(Mode::for_persona("sparkle"), Mode::Coding);
        assert_eq!(Mode::for_persona("Sparkle"), Mode::Coding);
        assert_eq!(Mode::for_persona("bolt"), Mode::Coding);
        assert_eq!(Mode::for_persona("codebeard"), Mode::Coding);
    }

    #[test]
    fn homelab_defaults_to_coding() {
        assert_eq!(Mode::for_persona("homelab"), Mode::Coding);
    }

    #[test]
    fn unknown_persona_defaults_to_coding() {
        assert_eq!(Mode::for_persona("unknown"), Mode::Coding);
    }

    #[test]
    fn creative_has_prompt_suffix() {
        assert!(Mode::Creative.prompt_suffix().is_some());
        assert!(Mode::Creative.prompt_suffix().unwrap().contains("Creative"));
    }

    #[test]
    fn coding_has_no_prompt_suffix() {
        assert!(Mode::Coding.prompt_suffix().is_none());
    }

    #[test]
    fn display_format() {
        assert_eq!(format!("{}", Mode::Coding), "coding");
        assert_eq!(format!("{}", Mode::Creative), "creative");
    }
}
