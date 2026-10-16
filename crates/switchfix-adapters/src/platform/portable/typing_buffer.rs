//! Shared keystroke buffer (RAM only) — portable.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use switchfix_app::ports::{TypingBufferPort, WordEvent};
use switchfix_domain::WordSpan;
use tokio::sync::broadcast;

/// Shared keystroke buffer (RAM only).
#[derive(Debug)]
pub struct SharedTypingBuffer {
    chars: Mutex<VecDeque<char>>,
    max: usize,
    enabled: AtomicBool,
    word_tx: broadcast::Sender<WordEvent>,
    /// Monotonic ms of the last physical key event (down or up).
    /// Used to wait until the user releases the completing key before
    /// injecting a word replace (arrow keys / backspace mid-press break).
    last_key_ms: AtomicU64,
    /// How many physical keys are currently held down. A held key does not
    /// generate events, so time-since-last-event alone is not enough.
    keys_down: AtomicUsize,
    started: Instant,
}

impl SharedTypingBuffer {
    pub fn new(max: usize, enabled: bool) -> Arc<Self> {
        let (word_tx, _) = broadcast::channel(64);
        Arc::new(Self {
            chars: Mutex::new(VecDeque::with_capacity(max.min(256))),
            max: max.max(1),
            enabled: AtomicBool::new(enabled),
            word_tx,
            last_key_ms: AtomicU64::new(0),
            keys_down: AtomicUsize::new(0),
            started: Instant::now(),
        })
    }

    /// Record a physical key event (call from the input loop on down and up).
    pub fn note_key_event(&self) {
        let ms = self.started.elapsed().as_millis() as u64;
        self.last_key_ms.store(ms, Ordering::SeqCst);
    }

    /// Physical key pressed (value=1).
    pub fn note_key_down(&self) {
        self.keys_down.fetch_add(1, Ordering::SeqCst);
        self.note_key_event();
    }

    /// Physical key released (value=0).
    pub fn note_key_up(&self) {
        self.keys_down
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            })
            .ok();
        self.note_key_event();
    }

    /// How many physical keys are held right now.
    pub fn keys_down(&self) -> usize {
        self.keys_down.load(Ordering::SeqCst)
    }

    /// How long ago was the last physical key event?
    pub fn ms_since_last_key(&self) -> u64 {
        let ms = self.started.elapsed().as_millis() as u64;
        let last = self.last_key_ms.load(Ordering::SeqCst);
        ms.saturating_sub(last)
    }

    /// Block until **no keys are held** and none has been seen for `quiet_ms`
    /// (give up after `max_ms`).
    pub async fn wait_keys_quiet(&self, quiet_ms: u64, max_ms: u64) {
        let deadline = Instant::now() + Duration::from_millis(max_ms);
        loop {
            let held = self.keys_down();
            let quiet = self.ms_since_last_key() >= quiet_ms;
            if held == 0 && quiet {
                return;
            }
            if Instant::now() >= deadline {
                tracing::debug!(
                    "wait_keys_quiet timeout (held={held}, since={})",
                    self.ms_since_last_key()
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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

        if Self::is_sep(c) {
            // Word just finished — emit Completed with the word before the separator.
            let word = Self::peek_word_locked(&g, true);
            if let Some(text) = word {
                let char_len = text.chars().count();
                let _ = self.word_tx.send(WordEvent::Completed {
                    word: WordSpan { text, char_len },
                    separator: c,
                });
            }
        } else {
            let frag = Self::peek_word_locked(&g, false).unwrap_or_default();
            let _ = self.word_tx.send(WordEvent::Updated(frag));
        }
    }

    pub fn pop(&self) {
        self.chars.lock().unwrap().pop_back();
        // Notify spell layer so it can undo a recent auto-replace.
        if self.enabled.load(Ordering::SeqCst) {
            let _ = self.word_tx.send(WordEvent::Backspace);
        }
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

    /// Last non-separator run. When `drop_trailing_sep` is set, the just-typed
    /// separator is ignored so we can recover the word that ended.
    fn peek_word_locked(g: &VecDeque<char>, drop_trailing_sep: bool) -> Option<String> {
        let mut end = g.len();
        if drop_trailing_sep {
            while end > 0 && Self::is_sep(g[end - 1]) {
                end -= 1;
            }
        }
        if end == 0 {
            return None;
        }
        let mut start = end;
        while start > 0 && !Self::is_sep(g[start - 1]) {
            start -= 1;
        }
        if start >= end {
            return None;
        }
        Some(g.range(start..end).collect())
    }

    pub fn peek_current_word(&self) -> Option<String> {
        if !self.enabled.load(Ordering::SeqCst) {
            return None;
        }
        let g = self.chars.lock().unwrap();
        Self::peek_word_locked(&g, false)
    }

    /// Replace buffer contents silently (no WordEvent). Keeps the next
    /// separator from completing a word we already replaced.
    pub fn set_contents(&self, text: &str) {
        let mut g = self.chars.lock().unwrap();
        g.clear();
        for c in text.chars() {
            if g.len() >= self.max {
                g.pop_front();
            }
            g.push_back(c);
        }
    }

    /// Last `max_chars` characters as a string (spell-backend context).
    pub fn peek_tail(&self, max_chars: usize) -> String {
        if !self.enabled.load(Ordering::SeqCst) {
            return String::new();
        }
        let g = self.chars.lock().unwrap();
        let skip = g.len().saturating_sub(max_chars);
        g.iter().skip(skip).collect()
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

#[async_trait::async_trait]
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
    fn peek_current_word(&self) -> Option<String> {
        SharedTypingBuffer::peek_current_word(self)
    }
    fn set_contents(&self, text: &str) {
        SharedTypingBuffer::set_contents(self, text);
    }
    fn peek_tail(&self, max_chars: usize) -> String {
        SharedTypingBuffer::peek_tail(self, max_chars)
    }
    async fn wait_keys_quiet(&self, quiet_ms: u64, max_ms: u64) {
        SharedTypingBuffer::wait_keys_quiet(self, quiet_ms, max_ms).await;
    }
    fn word_events(&self) -> broadcast::Receiver<WordEvent> {
        self.word_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn emits_completed_on_separator() {
        let b = SharedTypingBuffer::new(64, true);
        let mut rx = b.word_events();
        for c in "helo ".chars() {
            b.push(c);
        }
        let mut got = None;
        while let Ok(ev) = rx.try_recv() {
            if let WordEvent::Completed { word, separator } = ev {
                got = Some((word, separator));
            }
        }
        let (word, sep) = got.unwrap();
        assert_eq!(word.text, "helo");
        assert_eq!(sep, ' ');
    }

    #[tokio::test]
    async fn emits_updated_while_typing() {
        let b = SharedTypingBuffer::new(64, true);
        let mut rx = b.word_events();
        for c in "прив".chars() {
            b.push(c);
        }
        let mut last = None;
        while let Ok(ev) = rx.try_recv() {
            if let WordEvent::Updated(f) = ev {
                last = Some(f);
            }
        }
        assert_eq!(last.as_deref(), Some("прив"));
    }

    #[test]
    fn peek_current_word_reads_fragment() {
        let b = SharedTypingBuffer::new(64, true);
        for c in "hello wor".chars() {
            b.push(c);
        }
        assert_eq!(b.peek_current_word().as_deref(), Some("wor"));
    }

    #[test]
    fn set_contents_replaces_silently() {
        let b = SharedTypingBuffer::new(64, true);
        let mut rx = b.word_events();
        for c in "приветт ".chars() {
            b.push(c);
        }
        while rx.try_recv().is_ok() {}
        b.set_contents("привет ");
        // No new events.
        assert!(rx.try_recv().is_err());
        // Next separator must NOT re-complete "приветт".
        b.push(' ');
        let mut got = None;
        while let Ok(ev) = rx.try_recv() {
            if let WordEvent::Completed { word, .. } = ev {
                got = Some(word.text);
            }
        }
        assert_eq!(got.as_deref(), Some("привет"));
    }

    #[tokio::test]
    async fn wait_keys_quiet_requires_key_release() {
        let b = SharedTypingBuffer::new(64, true);
        b.note_key_down(); // space held — no new events while held
        tokio::time::sleep(Duration::from_millis(80)).await;
        // Held key: must NOT be quiet yet (old bug: time-since-event looked quiet).
        assert!(b.keys_down() == 1);
        let start = Instant::now();
        // Give it only a short max; it should hit the timeout while held.
        b.wait_keys_quiet(20, 80).await;
        assert!(start.elapsed() >= Duration::from_millis(50), "must wait while held");
        b.note_key_up();
        let start = Instant::now();
        b.wait_keys_quiet(20, 500).await;
        assert!(start.elapsed() < Duration::from_millis(200), "returns quickly after release");
    }
}
