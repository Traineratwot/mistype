//! Shared keystroke buffer (RAM only) — portable.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use switchfix_app::ports::TypingBufferPort;
use switchfix_domain::WordSpan;

/// Shared keystroke buffer (RAM only).
#[derive(Debug)]
pub struct SharedTypingBuffer {
    chars: Mutex<VecDeque<char>>,
    max: usize,
    enabled: AtomicBool,
}

impl SharedTypingBuffer {
    pub fn new(max: usize, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            chars: Mutex::new(VecDeque::with_capacity(max.min(256))),
            max: max.max(1),
            enabled: AtomicBool::new(enabled),
        })
    }

    pub fn push(&self, c: char) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        let mut g = self.chars.lock().unwrap();
        if g.len() >= self.max {
            g.pop_front();
        }
        g.push_back(c);
    }

    pub fn pop(&self) {
        self.chars.lock().unwrap().pop_back();
    }

    pub fn clear(&self) {
        self.chars.lock().unwrap().clear();
    }

    fn is_sep(c: char) -> bool {
        c.is_whitespace()
            || matches!(
                c,
                ',' | '.' | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '"'
                    | '\''
                    | '/'
                    | '\\'
            )
    }

    pub fn take_last_word(&self) -> Option<WordSpan> {
        if !self.enabled.load(Ordering::SeqCst) {
            return None;
        }
        let mut g = self.chars.lock().unwrap();

        while g.back().is_some_and(|c| Self::is_sep(*c)) {
            g.pop_back();
        }
        if g.is_empty() {
            return None;
        }

        let mut word = String::new();
        while let Some(c) = g.back() {
            if Self::is_sep(*c) {
                break;
            }
            word.insert(0, *c);
            g.pop_back();
        }

        if word.is_empty() {
            return None;
        }

        let char_len = word.chars().count();
        g.clear();
        Some(WordSpan { text: word, char_len })
    }
}

impl TypingBufferPort for SharedTypingBuffer {
    fn take_last_word(&self) -> Option<WordSpan> {
        SharedTypingBuffer::take_last_word(self)
    }
    fn clear(&self) {
        SharedTypingBuffer::clear(self);
    }
    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    fn push_char(&self, c: char) {
        self.push(c);
    }
    fn backspace(&self) {
        self.pop();
    }
}
