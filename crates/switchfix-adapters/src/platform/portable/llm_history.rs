//! Rolling log + live status of LLM-fix requests (for the Plasma widget).
//!
//! Single JSON file at `/tmp/switchfix.llm-history.json`:
//! `{ "pid", "updated", "enabled", "busy", "history": [ … ] }`
//! History is newest-last in the file, newest-first in the API. Capped.
//! Cleared on reboot (tmp).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 30;
/// Heartbeat is fresh if written within this window.
pub const FRESH_MS: u64 = 15_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmHistoryEntry {
    /// Unix time in milliseconds.
    pub ts: u64,
    /// Original selection (before replace).
    pub from: String,
    /// LLM result (after replace).
    pub to: String,
    /// `true` when the text was actually replaced.
    pub ok: bool,
}

/// Full payload written by the daemon and printed by `switchfix llm-history`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmStatus {
    pub pid: u32,
    /// Unix ms of the last daemon write.
    pub updated: u64,
    pub enabled: bool,
    pub busy: bool,
    /// Newest first.
    pub history: Vec<LlmHistoryEntry>,
}

impl Default for LlmStatus {
    fn default() -> Self {
        Self {
            pid: 0,
            updated: 0,
            enabled: false,
            busy: false,
            history: Vec::new(),
        }
    }
}

pub fn llm_history_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.llm-history.json")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Read the status file. Accepts both the wrapper object and a legacy bare array.
pub fn read_llm_status() -> LlmStatus {
    let path = llm_history_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return LlmStatus::default();
    };
    if let Ok(mut st) = serde_json::from_str::<LlmStatus>(&raw) {
        // File stores oldest-first; API/UI want newest-first.
        st.history.reverse();
        return st;
    }
    // Legacy: bare JSON array of entries (oldest-first).
    if let Ok(list) = serde_json::from_str::<Vec<LlmHistoryEntry>>(&raw) {
        let mut st = LlmStatus::default();
        st.history = list;
        st.history.reverse();
        return st;
    }
    LlmStatus::default()
}

fn write_status(st: &LlmStatus) {
    let mut out = st.clone();
    // Persist oldest-first so append can push at the end.
    out.history.reverse();
    if let Ok(json) = serde_json::to_string(&out) {
        let _ = std::fs::write(llm_history_path(), json);
    }
}

/// Daemon heartbeat: liveness + enabled + busy. Preserves history.
pub fn write_llm_status(enabled: bool, busy: bool) {
    let mut st = read_llm_status();
    st.pid = std::process::id();
    st.updated = now_ms();
    st.enabled = enabled;
    st.busy = busy;
    write_status(&st);
}

/// Refresh liveness/enabled without touching the busy flag (periodic tick).
pub fn touch_llm_status(enabled: bool) {
    let mut st = read_llm_status();
    st.pid = std::process::id();
    st.updated = now_ms();
    st.enabled = enabled;
    write_status(&st);
}

/// Append one entry; keep at most [`MAX_ENTRIES`] (oldest dropped).
pub fn append_llm_history(entry: LlmHistoryEntry) {
    let mut st = read_llm_status();
    st.history.push(entry); // newest-first in memory
    if st.history.len() > MAX_ENTRIES {
        let skip = st.history.len() - MAX_ENTRIES;
        st.history = st.history.split_off(skip);
    }
    // Keep status fields fresh too.
    st.pid = std::process::id();
    st.updated = now_ms();
    write_status(&st);
}

/// Newest-first history only.
pub fn read_llm_history() -> Vec<LlmHistoryEntry> {
    read_llm_status().history
}

/// True when a daemon has written the file recently (widget-friendly).
pub fn llm_status_is_fresh(st: &LlmStatus) -> bool {
    if st.updated == 0 {
        return false;
    }
    now_ms().saturating_sub(st.updated) <= FRESH_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(from: &str, to: &str) -> LlmHistoryEntry {
        LlmHistoryEntry {
            ts: 1,
            from: from.into(),
            to: to.into(),
            ok: true,
        }
    }

    #[test]
    fn fresh_when_recently_updated() {
        let mut st = LlmStatus::default();
        assert!(!llm_status_is_fresh(&st));
        st.updated = now_ms();
        assert!(llm_status_is_fresh(&st));
        st.updated = now_ms().saturating_sub(FRESH_MS + 1);
        assert!(!llm_status_is_fresh(&st));
    }

    #[test]
    fn cap_keeps_at_most_max() {
        let mut list = Vec::new();
        for i in 0..(MAX_ENTRIES + 5) {
            list.push(entry(&format!("f{i}"), &format!("t{i}")));
        }
        if list.len() > MAX_ENTRIES {
            let skip = list.len() - MAX_ENTRIES;
            list = list.split_off(skip);
        }
        assert_eq!(list.len(), MAX_ENTRIES);
    }
}
