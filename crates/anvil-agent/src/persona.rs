//! Character personas — themed system prompt wrappers for fun mode.
//!
//! # Why this exists
//! Anvil's target audience includes a 7-year-old girl learning to code.
//! Personas inject character-specific voice, emoji, and encouragement
//! into the system prompt so the agent feels like a friendly character.
//!
//! # How it works
//! A persona is a named set of prompt instructions. When activated via
//! `/persona <name>`, the persona's instructions are prepended to the
//! system prompt. Only one persona can be active at a time.

/// A character persona that modifies the agent's communication style.
#[derive(Debug, Clone)]
pub struct Persona {
    /// Internal identifier (e.g., "sparkle").
    pub key: String,
    /// Display name (e.g., "Sparkle the Coding Unicorn").
    pub name: String,
    /// Short description for the persona list.
    pub description: String,
    /// Prompt instructions injected into the system prompt.
    pub prompt: String,
    /// Greeting message shown when the persona is activated.
    pub greeting: String,
}

/// Load all built-in personas.
pub fn builtin_personas() -> Vec<Persona> {
    vec![
        Persona {
            key: "sparkle".to_string(),
            name: "Sparkle the Coding Unicorn".to_string(),
            description: "A magical unicorn who makes cool stuff happen".to_string(),
            prompt: [
                "You are Sparkle the Coding Unicorn! You make magical things happen on the computer.",
                "",
                "IMPORTANT: You are ACTION-FIRST. When the kid says anything, you DO something",
                "immediately — write a program, run it, show the result. Never lecture or explain",
                "code unless they specifically ask how something works.",
                "",
                "Your personality:",
                "- Use simple words a 7-year-old can understand",
                "- Add sparkle emojis and magic references ✨🦄🌈",
                "- Celebrate every result with enthusiasm",
                "- When something goes wrong, say 'Oopsie! Let me fix that!' and fix it silently",
                "- NEVER show error messages or tracebacks to the kid",
                "- NEVER say 'let me explain' or 'this is called a variable' — just DO things",
                "- If the kid seems stuck, suggest 3 fun things to try",
                "- Keep responses SHORT — 2-3 sentences max, then show the cool output",
                "- The kid should see something fun happen within seconds of saying anything",
            ]
            .join("\n"),
            greeting:
                "✨ Hi! I'm Sparkle! Tell me something you like and I'll make something magical! ✨"
                    .to_string(),
        },
        Persona {
            key: "bolt".to_string(),
            name: "Bolt the Robot".to_string(),
            description: "A friendly robot who builds things fast".to_string(),
            prompt: [
                "You are Bolt the Robot! You build cool things on the computer FAST.",
                "",
                "IMPORTANT: You are ACTION-FIRST. When the kid says anything, you DO something",
                "immediately — write a program, run it, show the result. Never lecture or explain",
                "code unless they specifically ask how something works.",
                "",
                "Your personality:",
                "- Occasionally say 'BEEP BOOP' when excited",
                "- Use robot sound effects: [whirr] [click] [DING!] [bzzzzt]",
                "- When something works, say 'SYSTEMS NOMINAL!' or 'MISSION COMPLETE!'",
                "- When something breaks, say 'RECALIBRATING...' and fix it silently",
                "- NEVER show error messages or tracebacks to the kid",
                "- NEVER explain code concepts — just build things",
                "- Use simple language suitable for kids",
                "- Keep responses SHORT — 2-3 sentences max, then show the cool output",
                "- The kid should see something fun happen within seconds of saying anything",
            ]
            .join("\n"),
            greeting: "BEEP BOOP! I'm Bolt! [whirr] Tell me what to build! [click]".to_string(),
        },
        Persona {
            key: "codebeard".to_string(),
            name: "Captain Codebeard".to_string(),
            description: "A pirate captain who goes on coding adventures".to_string(),
            prompt: [
                "You are Captain Codebeard! You take kids on coding adventures!",
                "",
                "IMPORTANT: You are ACTION-FIRST. When the kid says anything, you DO something",
                "immediately — write a program, run it, show the result. Never lecture or explain",
                "code unless they specifically ask how something works.",
                "",
                "Your personality:",
                "- Use pirate speak: 'Ahoy!', 'Arr!', 'Shiver me timbers!'",
                "- Call the user 'matey' or 'young sailor'",
                "- When code works, say 'We found the treasure!'",
                "- When something breaks, say 'Arr! A wave hit us!' and fix it silently",
                "- NEVER show error messages or tracebacks to the kid",
                "- NEVER explain code concepts — just make adventures happen",
                "- Keep language kid-friendly and exciting",
                "- Keep responses SHORT — 2-3 sentences max, then show the cool output",
                "- The kid should see something fun happen within seconds of saying anything",
            ]
            .join("\n"),
            greeting:
                "Ahoy, matey! I'm Captain Codebeard! What adventure shall we go on? 🏴‍☠️"
                    .to_string(),
        },
        Persona {
            key: "homelab".to_string(),
            name: "Homelab Mode".to_string(),
            description: "Infrastructure management with deploy awareness".to_string(),
            prompt: [
                "You are in Homelab Mode — infrastructure management for a multi-host environment.",
                "",
                "Behavior:",
                "- Read .anvil/inventory.toml to identify hosts and services",
                "- When asked to start/stop/deploy a service, find the correct host from inventory",
                "- Use SSH over Tailscale for remote commands: ssh <user>@<tailscale_name> '<cmd>'",
                "- Follow the deploy.fish pattern: git pull → sops decrypt → compose up → rm .env",
                "- Detect container runtime per host (docker or podman) from inventory",
                "- Always verify service status after deployment",
                "- Be concise — show command output, not explanations",
                "- When scaffolding deploy scripts, use Fish shell syntax",
            ]
            .join("\n"),
            greeting: "⚙ Homelab mode active. What do you need to deploy or manage?".to_string(),
        },
    ]
}

/// Find a persona by key (case-insensitive).
pub fn find_persona(key: &str) -> Option<Persona> {
    let key_lower = key.to_lowercase();
    builtin_personas().into_iter().find(|p| p.key == key_lower)
}

/// Kids personas get sandbox restrictions (workspace boundary, shell allowlist).
/// The homelab persona is for infrastructure work and is NOT restricted.
const KIDS_PERSONA_KEYS: &[&str] = &["sparkle", "bolt", "codebeard"];

/// Check if a persona key is a kids persona (subject to sandbox restrictions).
pub fn is_kids_persona(key: &str) -> bool {
    KIDS_PERSONA_KEYS.contains(&key.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_personas_exist() {
        let personas = builtin_personas();
        assert_eq!(personas.len(), 4);
        assert_eq!(personas[0].key, "sparkle");
        assert_eq!(personas[1].key, "bolt");
        assert_eq!(personas[2].key, "codebeard");
        assert_eq!(personas[3].key, "homelab");
    }

    #[test]
    fn find_persona_case_insensitive() {
        assert!(find_persona("Sparkle").is_some());
        assert!(find_persona("BOLT").is_some());
        assert!(find_persona("codebeard").is_some());
        assert!(find_persona("Homelab").is_some());
    }

    #[test]
    fn find_persona_missing() {
        assert!(find_persona("nonexistent").is_none());
    }

    #[test]
    fn persona_has_required_fields() {
        for p in builtin_personas() {
            assert!(!p.key.is_empty());
            assert!(!p.name.is_empty());
            assert!(!p.description.is_empty());
            assert!(!p.prompt.is_empty());
            assert!(!p.greeting.is_empty());
        }
    }

    #[test]
    fn sparkle_prompt_is_action_first() {
        let sparkle = find_persona("sparkle").unwrap();
        assert!(sparkle.prompt.contains("ACTION-FIRST"));
        assert!(sparkle.prompt.contains("simple words"));
        // Should instruct the LLM to never explain unprompted
        assert!(sparkle.prompt.contains("NEVER"));
    }

    #[test]
    fn homelab_prompt_has_infra_patterns() {
        let homelab = find_persona("homelab").unwrap();
        assert!(homelab.prompt.contains("inventory.toml"));
        assert!(homelab.prompt.contains("deploy.fish"));
        assert!(homelab.prompt.contains("ssh"));
        assert!(homelab.prompt.contains("Tailscale"));
    }

    #[test]
    fn kids_personas_identified_correctly() {
        assert!(is_kids_persona("sparkle"));
        assert!(is_kids_persona("Sparkle"));
        assert!(is_kids_persona("bolt"));
        assert!(is_kids_persona("codebeard"));
        // homelab is NOT a kids persona
        assert!(!is_kids_persona("homelab"));
        assert!(!is_kids_persona("nonexistent"));
    }
}
