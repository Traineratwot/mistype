//! Runtime spell/LLM status + transform history (for UI / `suggest-peek`).
//!
//! Written by the spell service; read by `switchfix suggest-peek` and the
//! Plasma widget. Kept in a small JSON file — no extra IPC.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// `/tmp/switchfix.spell.json` — status snapshot for UI.
pub fn spell_status_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.spell.json")
}

/// Append-only diagnostics log (recent transforms).
pub fn spell_history_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.spell-history.log")
}

const HISTORY_KEEP: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    /// ms since process start (stable, no clock skew in JSON).
    pub t_ms: u64,
    /// `replace` | `undo` | `hint` | `veto` | `llm_ok` | `llm_err`
    pub action: String,
    pub from: String,
    pub to: String,
    /// `layout` | `dict` | `edit1` | `context` | `none`
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SpellStatus {
    // --- LLM / context backend ---
    pub llm_configured: bool,
    /// `llm` / `languagetool` / empty
    pub llm_name: String,
    /// Last context call: `true` ok, `false` error, `null` never called.
    pub llm_ok: Option<bool>,
    pub llm_calls: u64,
    pub llm_errors: u64,
    /// Last LLM latency in ms (0 = unknown).
    pub llm_last_ms: u64,
    /// A context call is in flight (widget spinner).
    pub llm_busy: bool,

    // --- outcomes ---
    pub replaced: u64,
    pub undone: u64,
    pub hints: u64,
    pub vetoes: u64,

    // --- which layer produced the applied / shown fix ---
    pub via_layout: u64,
    pub via_dict: u64,
    pub via_edit1: u64,
    pub via_context: u64,

    pub history: VecDeque<HistoryEntry>,
}

/// Shared status handle (daemon writes, CLI/widget read the JSON file).
pub struct SpellStatusStore {
    inner: Mutex<SpellStatus>,
    started: std::time::Instant,
    pub llm_calls: AtomicU64,
    pub llm_errors: AtomicU64,
}

impl SpellStatusStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(SpellStatus::default()),
            started: std::time::Instant::now(),
            llm_calls: AtomicU64::new(0),
            llm_errors: AtomicU64::new(0),
        })
    }

    fn t_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn set_llm(&self, configured: bool, name: &str) {
        let mut g = self.inner.lock().unwrap();
        g.llm_configured = configured;
        g.llm_name = name.to_string();
        self.persist(&g);
    }

    pub fn note_llm(&self, ok: bool, ms: u64) {
        self.llm_calls.fetch_add(1, Ordering::Relaxed);
        if !ok {
            self.llm_errors.fetch_add(1, Ordering::Relaxed);
        }
        let mut g = self.inner.lock().unwrap();
        g.llm_ok = Some(ok);
        g.llm_calls = self.llm_calls.load(Ordering::Relaxed);
        g.llm_errors = self.llm_errors.load(Ordering::Relaxed);
        g.llm_last_ms = ms;
        g.llm_busy = false;
        self.persist(&g);
    }

    /// Mark the context backend as in-flight (clears on drop of the guard).
    pub fn llm_busy(self: &Arc<Self>) -> LlmBusyGuard {
        self.set_llm_busy(true);
        LlmBusyGuard {
            store: Arc::clone(self),
        }
    }

    fn set_llm_busy(&self, busy: bool) {
        let mut g = self.inner.lock().unwrap();
        if g.llm_busy == busy {
            return;
        }
        g.llm_busy = busy;
        self.persist(&g);
    }

    /// Record a transform / decision and append to the diagnostics log.
    pub fn note(&self, action: &str, from: &str, to: &str, via: &str) {
        let mut g = self.inner.lock().unwrap();
        match action {
            "replace" => {
                g.replaced += 1;
                self.bump_via(&mut g, via);
            }
            "undo" => g.undone += 1,
            "hint" => {
                g.hints += 1;
                self.bump_via(&mut g, via);
            }
            "veto" => g.vetoes += 1,
            _ => {}
        }
        let entry = HistoryEntry {
            t_ms: self.t_ms(),
            action: action.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            via: via.to_string(),
        };
        // File log for diagnostics (append, cap soft).
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(spell_history_path())
        {
            use std::io::Write;
            let _ = writeln!(
                f,
                "[{}] {} {:?} -> {:?} via={}",
                entry.t_ms, entry.action, entry.from, entry.to, entry.via
            );
        }
        g.history.push_back(entry);
        while g.history.len() > HISTORY_KEEP {
            g.history.pop_front();
        }
        self.persist(&g);
    }

    fn bump_via(&self, g: &mut SpellStatus, via: &str) {
        match via {
            "layout" => g.via_layout += 1,
            "dict" => g.via_dict += 1,
            "edit1" => g.via_edit1 += 1,
            "context" => g.via_context += 1,
            _ => {}
        }
    }

    pub fn snapshot(&self) -> SpellStatus {
        self.inner.lock().unwrap().clone()
    }

    fn persist(&self, st: &SpellStatus) {
        if let Ok(json) = serde_json::to_string(st) {
            let _ = std::fs::write(spell_status_path(), json);
        }
    }
}

/// Clears `llm_busy` even if the context future is dropped mid-call.
pub struct LlmBusyGuard {
    store: Arc<SpellStatusStore>,
}

impl Drop for LlmBusyGuard {
    fn drop(&mut self) {
        self.store.set_llm_busy(false);
    }
}

/// Read the last written status (for CLI / tests).
pub fn read_spell_status() -> Option<SpellStatus> {
    let json = std::fs::read_to_string(spell_status_path()).ok()?;
    serde_json::from_str(&json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_counts_and_history() {
        let s = SpellStatusStore::new();
        let _ = std::fs::remove_file(spell_status_path());
        s.set_llm(true, "llm");
        s.note("replace", "приветт", "привет", "context");
        s.note("replace", "helo", "hello", "edit1");
        s.note("undo", "привет", "приветт", "none");
        s.note("hint", "abc", "abcd", "dict");
        let snap = s.snapshot();
        assert_eq!(snap.replaced, 2);
        assert_eq!(snap.undone, 1);
        assert_eq!(snap.hints, 1);
        assert_eq!(snap.via_context, 1);
        assert_eq!(snap.via_edit1, 1);
        assert_eq!(snap.history.len(), 4);
        assert_eq!(snap.history.back().unwrap().action, "hint");
        assert!(snap.llm_configured);
        assert_eq!(snap.llm_name, "llm");
        let _ = std::fs::remove_file(spell_status_path());
    }

    #[test]
    fn note_llm_tracks_errors() {
        let s = SpellStatusStore::new();
        let _ = std::fs::remove_file(spell_status_path());
        s.note_llm(true, 40);
        s.note_llm(false, 90);
        let snap = s.snapshot();
        assert_eq!(snap.llm_calls, 2);
        assert_eq!(snap.llm_errors, 1);
        assert_eq!(snap.llm_ok, Some(false));
        assert!(!snap.llm_busy);
        let _ = std::fs::remove_file(spell_status_path());
    }

    #[test]
    fn llm_busy_guard_sets_and_clears() {
        let s = SpellStatusStore::new();
        let _ = std::fs::remove_file(spell_status_path());
        {
            let _g = s.llm_busy();
            assert!(s.snapshot().llm_busy);
        }
        assert!(!s.snapshot().llm_busy);
        // note_llm also clears the flag (end of a call without a guard).
        let _g = s.llm_busy();
        s.note_llm(true, 10);
        assert!(!s.snapshot().llm_busy);
        let _ = std::fs::remove_file(spell_status_path());
    }
}
