use std::io::Write;
use std::path::PathBuf;

const DEFAULT_MAX_LINES: usize = 200;
const DEFAULT_MAX_BYTES: usize = 30_000;

#[derive(Clone)]
pub struct TruncationConfig {
    pub max_lines: usize,
    pub max_bytes: usize,
}

impl Default for TruncationConfig {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_MAX_LINES,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

pub struct TruncationResult {
    pub content: String,
    pub was_truncated: bool,
    pub temp_file: Option<PathBuf>,
}

/// Tail-truncate output by line count and byte size.
/// If truncated, saves full output to a temp file and appends a notice.
pub fn truncate_output(output: &str, config: &TruncationConfig) -> TruncationResult {
    let lines: Vec<&str> = output.lines().collect();
    let total_lines = lines.len();
    let total_bytes = output.len();

    let needs_line_truncation = total_lines > config.max_lines;
    let needs_byte_truncation = total_bytes > config.max_bytes;

    if !needs_line_truncation && !needs_byte_truncation {
        return TruncationResult {
            content: output.to_string(),
            was_truncated: false,
            temp_file: None,
        };
    }

    // Save full output to temp file
    let temp_path = save_to_temp_file(output);

    // Take the tail lines
    let tail_lines = if needs_line_truncation {
        &lines[total_lines - config.max_lines..]
    } else {
        &lines[..]
    };

    let mut truncated = tail_lines.join("\n");

    // Further truncate by bytes if needed
    if truncated.len() > config.max_bytes {
        // Find a line boundary within the byte limit
        let mut byte_count = 0;
        let mut start_idx = truncated.lines().count();
        for (i, line) in truncated.lines().rev().enumerate() {
            byte_count += line.len() + 1; // +1 for newline
            if byte_count > config.max_bytes {
                start_idx = truncated.lines().count() - i;
                break;
            }
        }
        let kept_lines: Vec<&str> = truncated.lines().skip(start_idx).collect();
        truncated = kept_lines.join("\n");
    }

    let shown_lines = truncated.lines().count();
    let start_line = total_lines - shown_lines + 1;

    let notice = match &temp_path {
        Some(path) => format!(
            "\n\n[Showing lines {}-{} of {}. Full output: {}]",
            start_line,
            total_lines,
            total_lines,
            path.display()
        ),
        None => format!(
            "\n\n[Showing lines {}-{} of {}. Full output could not be saved.]",
            start_line, total_lines, total_lines
        ),
    };
    truncated.push_str(&notice);

    TruncationResult {
        content: truncated,
        was_truncated: true,
        temp_file: temp_path,
    }
}

fn save_to_temp_file(content: &str) -> Option<PathBuf> {
    let id = uuid::Uuid::new_v4();
    let path = std::env::temp_dir().join(format!("anvil-{}.log", id));
    match std::fs::File::create(&path) {
        Ok(mut f) => {
            if f.write_all(content.as_bytes()).is_ok() {
                Some(path)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_truncation_for_short_output() {
        let output = "line1\nline2\nline3";
        let result = truncate_output(output, &TruncationConfig::default());
        assert!(!result.was_truncated);
        assert_eq!(result.content, output);
        assert!(result.temp_file.is_none());
    }

    #[test]
    fn truncates_by_line_count() {
        let config = TruncationConfig {
            max_lines: 3,
            max_bytes: 100_000,
        };
        let lines: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        let output = lines.join("\n");

        let result = truncate_output(&output, &config);
        assert!(result.was_truncated);
        assert!(result.content.contains("line 8"));
        assert!(result.content.contains("line 9"));
        assert!(result.content.contains("line 10"));
        assert!(!result.content.starts_with("line 1\n"));
        assert!(result.content.contains("[Showing lines"));
        assert!(result.temp_file.is_some());

        // Clean up temp file
        if let Some(path) = result.temp_file {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn truncates_by_byte_size() {
        let config = TruncationConfig {
            max_lines: 10_000,
            max_bytes: 50,
        };
        let lines: Vec<String> = (1..=20)
            .map(|i| format!("this is line number {i}"))
            .collect();
        let output = lines.join("\n");

        let result = truncate_output(&output, &config);
        assert!(result.was_truncated);
        assert!(result.content.len() < output.len());
        assert!(result.temp_file.is_some());

        if let Some(path) = result.temp_file {
            let saved = std::fs::read_to_string(&path).unwrap();
            assert_eq!(saved, output);
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn temp_file_contains_full_output() {
        let config = TruncationConfig {
            max_lines: 2,
            max_bytes: 100_000,
        };
        let output = "line1\nline2\nline3\nline4\nline5";
        let result = truncate_output(output, &config);

        assert!(result.temp_file.is_some());
        let saved = std::fs::read_to_string(result.temp_file.as_ref().unwrap()).unwrap();
        assert_eq!(saved, output);

        if let Some(path) = result.temp_file {
            let _ = std::fs::remove_file(path);
        }
    }
}
