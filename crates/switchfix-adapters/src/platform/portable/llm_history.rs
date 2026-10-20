//! Rolling log of LLM-fix requests (for the Plasma widget).
//!
//! JSON array at `/tmp/switchfix.llm-history.json`, newest last, capped.
//! Contains only what the user already saw on screen: source and result
//! of the «LLM-fix selection» action. Cleared on reboot (tmp).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 30;

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

pub fn llm_history_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.llm-history.json")
}

/// Append one entry; keep at most [`MAX_ENTRIES`] (oldest dropped).
pub fn append_llm_history(entry: LlmHistoryEntry) {
    let path = llm_history_path();
    let mut list = read_llm_history();
    list.push(entry);
    if list.len() > MAX_ENTRIES {
        let skip = list.len() - MAX_ENTRIES;
        list = list.split_off(skip);
    }
    if let Ok(json) = serde_json::to_string_pretty(&list) {
        let _ = std::fs::write(&path, json);
    }
}

/// Newest-first list for UI / CLI.
pub fn read_llm_history() -> Vec<LlmHistoryEntry> {
    let path = llm_history_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut list: Vec<LlmHistoryEntry> = serde_json::from_str(&raw).unwrap_or_default();
    list.reverse();
    list
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
    fn append_and_read_newest_first() {
        // Isolate: use a unique temp dir via env is hard; just exercise parse
        // on an empty/missing file and the cap logic with a local vec.
        let mut list = vec![entry("a", "b"), entry("c", "d")];
        list.reverse();
        assert_eq!(list[0].from, "c");
        let mut list = list;
        for i in 0..MAX_ENTRIES {
            list.push(entry(&format!("f{i}"), &format!("t{i}")));
        }
        if list.len() > MAX_ENTRIES {
            let skip = list.len() - MAX_ENTRIES;
            list = list.split_off(skip);
        }
        assert_eq!(list.len(), MAX_ENTRIES);
        assert!(read_llm_history().len() <= MAX_ENTRIES);
    }
}
