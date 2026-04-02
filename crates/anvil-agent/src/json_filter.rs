//! JSON extraction filter for persona-contaminated LLM output.
//!
//! # Why this exists
//! Local LLMs with active personas sometimes wrap tool call arguments
//! in conversational text (persona bleed). For example:
//! ```text
//! Arr matey! Here be the command: {"command": "ls -la"}
//! ```
//! The OpenAI API spec says `arguments` is a JSON string, but small
//! local models don't always comply. This filter extracts the JSON
//! object from the surrounding text before parsing.
//!
//! Think of it as a fuel filter on an intake line — the LLM output is
//! the raw fuel, and we need to strip debris (persona text) before it
//! reaches the injector (serde_json).

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// Extract a JSON object from potentially contaminated LLM output.
///
/// Tries clean parse first (zero overhead on the happy path). If that
/// fails, uses regex to find the outermost `{...}` and retries.
/// Falls back to an empty JSON object if nothing works.
pub fn extract_json(raw: &str) -> Value {
    // Happy path: clean JSON parses directly
    if let Ok(val) = serde_json::from_str(raw) {
        return val;
    }

    // Regex extraction: find outermost { ... }
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?s)\{.*\}").expect("json filter regex"));

    if let Some(m) = re.find(raw) {
        if let Ok(val) = serde_json::from_str(m.as_str()) {
            tracing::debug!(
                "json_filter: extracted JSON from persona-contaminated output ({} bytes stripped)",
                raw.len() - m.as_str().len()
            );
            return val;
        }
    }

    tracing::debug!("json_filter: no valid JSON found, falling back to empty object");
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_json_parses_directly() {
        let input = r#"{"command": "ls -la"}"#;
        let val = extract_json(input);
        assert_eq!(val["command"], "ls -la");
    }

    #[test]
    fn extracts_json_from_persona_bleed() {
        let input = r#"Arr matey! Here be the command: {"command": "ls -la"}"#;
        let val = extract_json(input);
        assert_eq!(val["command"], "ls -la");
    }

    #[test]
    fn extracts_nested_json() {
        let input = r#"Sure! {"a": {"b": 1}, "c": 2}"#;
        let val = extract_json(input);
        assert_eq!(val["a"]["b"], 1);
        assert_eq!(val["c"], 2);
    }

    #[test]
    fn no_json_returns_empty_object() {
        let input = "no json here at all";
        let val = extract_json(input);
        assert!(val.is_object());
        assert!(val.as_object().unwrap().is_empty());
    }

    #[test]
    fn empty_string_returns_empty_object() {
        let val = extract_json("");
        assert!(val.is_object());
        assert!(val.as_object().unwrap().is_empty());
    }

    #[test]
    fn sparkle_persona_bleed() {
        let input = r#"✨ Ooh, let me look at that file for you! {"path": "main.rs"} ✨"#;
        let val = extract_json(input);
        assert_eq!(val["path"], "main.rs");
    }

    #[test]
    fn multiline_json_extraction() {
        let input = "Here's what I found:\n{\n  \"command\": \"cargo test\",\n  \"timeout\": 30\n}\nHope that helps!";
        let val = extract_json(input);
        assert_eq!(val["command"], "cargo test");
        assert_eq!(val["timeout"], 30);
    }
}
