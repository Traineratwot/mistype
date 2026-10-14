//! In-memory keystroke buffer used for "convert last word" without clipboard.

use crate::model::LayoutId;

/// A word span captured from the typing buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSpan {
    pub text: String,
    /// Number of chars (grapheme-agnostic, uses `chars().count()`) to erase via backspace.
    pub char_len: usize,
}

/// Ring buffer of recently typed characters (RAM only, never persisted).
#[derive(Debug)]
pub struct TypingBuffer {
    chars: Vec<char>,
    max: usize,
    enabled: bool,
    /// Word separators.
    separators: Vec<char>,
}

impl Default for TypingBuffer {
    fn default() -> Self {
        Self::new(256)
    }
}

impl TypingBuffer {
    pub fn new(max: usize) -> Self {
        Self {
            chars: Vec::with_capacity(max.min(256)),
            max: max.max(1),
            enabled: true,
            separators: vec![' ', '\t', '\n', '\r', ',', '.', ';', ':', '!', '?', '(', ')', '[', ']', '{', '}', '"', '\'', '/', '\\'],
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Push a character the user just typed.
    pub fn push(&mut self, c: char) {
        if !self.enabled {
            return;
        }
        if self.chars.len() >= self.max {
            self.chars.remove(0);
        }
        self.chars.push(c);
    }

    /// Backspace: drop the last char (if any).
    pub fn backspace(&mut self) {
        self.chars.pop();
    }

    /// Ctrl+Backspace / word-delete: drop the last word including trailing separators before it.
    pub fn delete_word(&mut self) {
        while self
            .chars
            .last()
            .is_some_and(|c| self.separators.contains(c))
        {
            self.chars.pop();
        }
        while self
            .chars
            .last()
            .is_some_and(|c| !self.separators.contains(c))
        {
            self.chars.pop();
        }
    }

    /// Take the last word from the buffer and clear it (so a second hotkey is a no-op).
    pub fn take_last_word(&mut self) -> Option<WordSpan> {
        if !self.enabled {
            return None;
        }

        // Ignore trailing separators at the end (user already typed space after the word).
        let end = self
            .chars
            .iter()
            .rposition(|c| !self.separators.contains(c))
            .map(|i| i + 1)?;

        let start = self.chars[..end]
            .iter()
            .rposition(|c| self.separators.contains(c))
            .map(|i| i + 1)
            .unwrap_or(0);

        if start >= end {
            return None;
        }

        let text: String = self.chars[start..end].iter().collect();
        let char_len = end - start;

        // Clear buffer after take (user expects one-shot conversion).
        self.chars.clear();

        if text.is_empty() {
            return None;
        }

        Some(WordSpan { text, char_len })
    }

    /// Peek at the last word without consuming the buffer (mainly for tests).
    pub fn peek_last_word(&self) -> Option<String> {
        let end = self
            .chars
            .iter()
            .rposition(|c| !self.separators.contains(c))
            .map(|i| i + 1)?;
        let start = self.chars[..end]
            .iter()
            .rposition(|c| self.separators.contains(c))
            .map(|i| i + 1)
            .unwrap_or(0);
        if start >= end {
            return None;
        }
        Some(self.chars[start..end].iter().collect())
    }

    pub fn clear(&mut self) {
        self.chars.clear();
    }

    /// Feed a full string (helper for tests / paste paths). Layout is ignored —
    /// the buffer stores the *characters the user intended* after conversion
    /// or the raw keystrokes' characters from the active layout.
    pub fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            self.push(c);
        }
    }

    /// Map a physical-layout character through a conversion table's inverse is
    /// done at the conversion layer; this is a convenience for tests.
    pub fn push_typed_as(&mut self, s: &str, _layout: &LayoutId) {
        self.push_str(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_word_simple() {
        let mut b = TypingBuffer::new(64);
        b.push_str("hello");
        let w = b.take_last_word().unwrap();
        assert_eq!(w.text, "hello");
        assert_eq!(w.char_len, 5);
        assert!(b.is_empty());
    }

    #[test]
    fn last_word_after_space() {
        let mut b = TypingBuffer::new(64);
        b.push_str("hello world");
        let w = b.take_last_word().unwrap();
        assert_eq!(w.text, "world");
        assert_eq!(w.char_len, 5);
    }

    #[test]
    fn last_word_ignores_trailing_space() {
        let mut b = TypingBuffer::new(64);
        b.push_str("привет ");
        let w = b.take_last_word().unwrap();
        assert_eq!(w.text, "привет");
        assert_eq!(w.char_len, 6);
    }

    #[test]
    fn last_word_punctuation_boundary() {
        let mut b = TypingBuffer::new(64);
        b.push_str("foo,bar");
        // last word is "bar" (',' is a separator)
        let w = b.take_last_word().unwrap();
        assert_eq!(w.text, "bar");
    }

    #[test]
    fn empty_buffer_returns_none() {
        let mut b = TypingBuffer::new(64);
        assert!(b.take_last_word().is_none());
    }

    #[test]
    fn only_separators_returns_none() {
        let mut b = TypingBuffer::new(64);
        b.push_str("   ");
        assert!(b.take_last_word().is_none());
    }

    #[test]
    fn backspace_removes_char() {
        let mut b = TypingBuffer::new(64);
        b.push_str("abc");
        b.backspace();
        assert_eq!(b.peek_last_word().as_deref(), Some("ab"));
    }

    #[test]
    fn delete_word_removes_word() {
        let mut b = TypingBuffer::new(64);
        b.push_str("hello world");
        b.delete_word();
        assert_eq!(b.peek_last_word().as_deref(), Some("hello"));
    }

    #[test]
    fn overflow_evicts_oldest() {
        let mut b = TypingBuffer::new(4);
        b.push_str("abcdef");
        assert_eq!(b.len(), 4);
        assert_eq!(b.peek_last_word().as_deref(), Some("cdef"));
    }

    #[test]
    fn disabled_buffer_returns_none() {
        let mut b = TypingBuffer::new(64).with_enabled(false);
        b.push_str("hello");
        assert!(b.take_last_word().is_none());
    }

    #[test]
    fn unicode_chars_counted() {
        let mut b = TypingBuffer::new(64);
        b.push_str("привет");
        let w = b.take_last_word().unwrap();
        assert_eq!(w.char_len, 6);
    }

    #[test]
    fn take_clears_so_second_is_none() {
        let mut b = TypingBuffer::new(64);
        b.push_str("word");
        assert!(b.take_last_word().is_some());
        assert!(b.take_last_word().is_none());
    }

    #[test]
    fn clear_resets() {
        let mut b = TypingBuffer::new(64);
        b.push_str("word");
        b.clear();
        assert!(b.is_empty());
        assert!(b.take_last_word().is_none());
    }
}
