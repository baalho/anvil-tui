//! Thinking mode filter — strips `<think>` blocks from LLM output.
//!
//! # Why this exists
//! Qwen3 and DeepSeek-R1 output chain-of-thought reasoning inside `<think>...</think>`
//! blocks. This is useful for debugging but clutters normal output. The filter
//! strips these blocks from displayed content while preserving them in session history.
//!
//! # How it works
//! The filter is a state machine that tracks whether we're inside a `<think>` block.
//! Content deltas arrive as small chunks (often mid-tag), so the filter buffers
//! partial tags and only emits content once it knows whether it's thinking or not.
//!
//! # Edge cases handled
//! - `<think>` split across chunks: `<thi` + `nk>`
//! - `</think>` split across chunks: `</thi` + `nk>`
//! - Multiple `<think>` blocks in one response
//! - Malformed/unclosed tags (treated as regular content after timeout)

/// Filters `<think>` blocks from streaming content deltas.
///
/// Call `push()` with each content delta. It returns the content that should
/// be displayed (with thinking blocks removed). Call `flush()` at the end
/// of a stream to emit any buffered content.
#[derive(Debug)]
pub struct ThinkingFilter {
    /// Whether we're currently inside a `<think>` block.
    in_thinking: bool,
    /// Buffer for partial tag detection (e.g., `<thi` waiting for `nk>`).
    tag_buf: String,
    /// Accumulated thinking content for the current block.
    thinking_content: String,
    /// Whether to show thinking blocks (toggled by `/think`).
    show_thinking: bool,
}

/// Result of pushing a content delta through the filter.
#[derive(Debug)]
pub struct FilterResult {
    /// Content to display to the user (empty if inside a hidden thinking block).
    pub display: String,
    /// Thinking content emitted (non-empty only when show_thinking is true
    /// or when a thinking block completes).
    pub thinking: String,
}

impl ThinkingFilter {
    pub fn new() -> Self {
        Self {
            in_thinking: false,
            tag_buf: String::new(),
            thinking_content: String::new(),
            show_thinking: false,
        }
    }

    /// Toggle whether thinking blocks are shown in output.
    pub fn set_show_thinking(&mut self, show: bool) {
        self.show_thinking = show;
    }

    pub fn show_thinking(&self) -> bool {
        self.show_thinking
    }

    /// Process a content delta and return what should be displayed.
    pub fn push(&mut self, delta: &str) -> FilterResult {
        let mut display = String::new();
        let mut thinking = String::new();

        // Append delta to tag buffer for processing
        self.tag_buf.push_str(delta);

        while !self.tag_buf.is_empty() {
            if self.in_thinking {
                // Look for </think>
                if let Some(end_pos) = self.tag_buf.find("</think>") {
                    // Everything before </think> is thinking content
                    let think_part = &self.tag_buf[..end_pos];
                    self.thinking_content.push_str(think_part);
                    if self.show_thinking {
                        thinking.push_str(think_part);
                    }

                    // Consume the closing tag
                    let after = self.tag_buf[end_pos + 8..].to_string();
                    self.tag_buf = after;
                    self.in_thinking = false;
                    self.thinking_content.clear();
                } else if self.tag_buf.len() > 8 && !could_be_partial_close(&self.tag_buf) {
                    // No closing tag and buffer is long enough that we're not
                    // waiting for a partial </think>. Emit as thinking content.
                    let content = std::mem::take(&mut self.tag_buf);
                    self.thinking_content.push_str(&content);
                    if self.show_thinking {
                        thinking.push_str(&content);
                    }
                    break;
                } else {
                    // Could be a partial </think> tag — wait for more data
                    break;
                }
            } else {
                // Look for <think>
                if let Some(start_pos) = self.tag_buf.find("<think>") {
                    // Everything before <think> is regular content
                    let before = &self.tag_buf[..start_pos];
                    display.push_str(before);

                    // Consume the opening tag
                    let after = self.tag_buf[start_pos + 7..].to_string();
                    self.tag_buf = after;
                    self.in_thinking = true;
                } else if let Some(partial_pos) = partial_open_position(&self.tag_buf) {
                    // Emit everything before the partial tag, keep the rest buffered
                    let before = &self.tag_buf[..partial_pos];
                    display.push_str(before);
                    let rest = self.tag_buf[partial_pos..].to_string();
                    self.tag_buf = rest;
                    break;
                } else {
                    // No tag found, emit everything as display content
                    let content = std::mem::take(&mut self.tag_buf);
                    display.push_str(&content);
                    break;
                }
            }
        }

        FilterResult { display, thinking }
    }

    /// Flush any remaining buffered content at end of stream.
    pub fn flush(&mut self) -> FilterResult {
        let mut display = String::new();
        let thinking = String::new();

        if !self.tag_buf.is_empty() {
            if self.in_thinking {
                // Unclosed <think> block — the buffered content was thinking
                // Don't display it (it's partial thinking)
                self.tag_buf.clear();
            } else {
                // Partial tag that never completed — emit as regular content
                display = std::mem::take(&mut self.tag_buf);
            }
        }

        self.in_thinking = false;
        self.thinking_content.clear();

        FilterResult { display, thinking }
    }
}

impl Default for ThinkingFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if the end of the buffer could be the start of `</think>`.
fn could_be_partial_close(buf: &str) -> bool {
    const CLOSE_TAG: &str = "</think>";
    for i in 1..CLOSE_TAG.len() {
        if buf.ends_with(&CLOSE_TAG[..i]) {
            return true;
        }
    }
    false
}

/// Find the byte position where a partial `<think>` tag starts at the end of the buffer.
/// Returns `None` if the buffer doesn't end with a partial open tag.
fn partial_open_position(buf: &str) -> Option<usize> {
    const OPEN_TAG: &str = "<think>";
    for i in (1..OPEN_TAG.len()).rev() {
        if buf.ends_with(&OPEN_TAG[..i]) {
            return Some(buf.len() - i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thinking_blocks() {
        let mut filter = ThinkingFilter::new();
        let r = filter.push("Hello, world!");
        assert_eq!(r.display, "Hello, world!");
        assert!(r.thinking.is_empty());
    }

    #[test]
    fn simple_thinking_block() {
        let mut filter = ThinkingFilter::new();
        let r = filter.push("<think>reasoning here</think>The answer is 42.");
        assert_eq!(r.display, "The answer is 42.");
        assert!(r.thinking.is_empty()); // show_thinking is false
    }

    #[test]
    fn thinking_block_shown_when_enabled() {
        let mut filter = ThinkingFilter::new();
        filter.set_show_thinking(true);
        let r = filter.push("<think>reasoning</think>answer");
        assert_eq!(r.display, "answer");
        assert_eq!(r.thinking, "reasoning");
    }

    #[test]
    fn thinking_block_split_across_chunks() {
        let mut filter = ThinkingFilter::new();

        let r1 = filter.push("<thi");
        assert!(r1.display.is_empty()); // buffered, waiting for more

        let r2 = filter.push("nk>I am thinking</think>done");
        assert_eq!(r2.display, "done");
    }

    #[test]
    fn close_tag_split_across_chunks() {
        let mut filter = ThinkingFilter::new();

        let r1 = filter.push("<think>thinking content</thi");
        assert!(r1.display.is_empty());

        let r2 = filter.push("nk>visible content");
        assert_eq!(r2.display, "visible content");
    }

    #[test]
    fn multiple_thinking_blocks() {
        let mut filter = ThinkingFilter::new();
        let r = filter.push("A<think>t1</think>B<think>t2</think>C");
        assert_eq!(r.display, "ABC");
    }

    #[test]
    fn content_before_thinking() {
        let mut filter = ThinkingFilter::new();
        let r = filter.push("prefix <think>hidden</think> suffix");
        assert_eq!(r.display, "prefix  suffix");
    }

    #[test]
    fn unclosed_thinking_block() {
        let mut filter = ThinkingFilter::new();
        let r1 = filter.push("<think>this never closes");
        assert!(r1.display.is_empty());

        let r2 = filter.flush();
        assert!(r2.display.is_empty()); // unclosed thinking is discarded
    }

    #[test]
    fn partial_open_tag_not_completed() {
        let mut filter = ThinkingFilter::new();
        let r1 = filter.push("hello <thi");
        // "hello " is emitted, "<thi" is buffered
        assert_eq!(r1.display, "hello ");

        // Now send something that doesn't complete the tag
        let r2 = filter.push("s is not a tag");
        assert_eq!(r2.display, "<this is not a tag");
    }

    #[test]
    fn flush_emits_buffered_non_thinking() {
        let mut filter = ThinkingFilter::new();
        let r1 = filter.push("hello <thi");
        assert_eq!(r1.display, "hello ");

        let r2 = filter.flush();
        assert_eq!(r2.display, "<thi");
    }

    #[test]
    fn empty_thinking_block() {
        let mut filter = ThinkingFilter::new();
        let r = filter.push("<think></think>content");
        assert_eq!(r.display, "content");
    }

    #[test]
    fn thinking_at_end_of_stream() {
        let mut filter = ThinkingFilter::new();
        let r1 = filter.push("answer<think>reasoning");
        assert_eq!(r1.display, "answer");

        let r2 = filter.flush();
        assert!(r2.display.is_empty());
    }
}
