//! Map physical keycodes to the **actual characters of the current layout**.
//!
//! evdev delivers US-QWERTY keycodes. When the user types in the RU layout,
//! keycode 25 (`p` in US) produces `з` on screen — the typing buffer must
//! store `з`, not `p`, or spellcheck will treat Russian words as English.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use mistype_domain::en_to_ru_map;

/// Layout-aware character mapper (shared with the evdev loop).
pub struct LayoutCharMapper {
    /// Short layout ids in KDE order, e.g. `["us", "ru"]`.
    layout_ids: Mutex<Vec<String>>,
    /// Current layout index (0-based, as `org.kde.KeyboardLayouts.getLayout`).
    current: AtomicU32,
    /// US-QWERTY char → RU-ЙЦУКЕН char (includes Shift variants).
    us_to_ru: HashMap<char, char>,
}

impl LayoutCharMapper {
    pub fn new(layout_ids: Vec<String>, current_index: u32) -> Self {
        Self {
            layout_ids: Mutex::new(layout_ids),
            current: AtomicU32::new(current_index),
            us_to_ru: en_to_ru_map(),
        }
    }

    /// Empty mapper (US passthrough) — used when no layout info is available.
    pub fn us_only() -> Self {
        Self::new(vec!["us".into()], 0)
    }

    pub fn set_layouts(&self, ids: Vec<String>) {
        *self.layout_ids.lock().unwrap() = ids;
    }

    pub fn set_current(&self, index: u32) {
        self.current.store(index, Ordering::SeqCst);
    }

    /// Current layout short id (`"us"`, `"ru"`, …). Falls back to `"us"`.
    pub fn current_id(&self) -> String {
        let ids = self.layout_ids.lock().unwrap();
        let idx = self.current.load(Ordering::SeqCst) as usize;
        ids.get(idx).cloned().unwrap_or_else(|| "us".into())
    }

    /// Convert a US-latin char (from `key_to_char`) into the character the
    /// current layout would actually produce.
    pub fn map_char(&self, latin: char) -> char {
        let id = self.current_id();
        if id.starts_with("ru") {
            self.us_to_ru.get(&latin).copied().unwrap_or(latin)
        } else {
            latin
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ru_layout_maps_us_keys_to_cyrillic() {
        let m = LayoutCharMapper::new(vec!["us".into(), "ru".into()], 1);
        // Physical keys for "ghbdtn" → "привет"
        let typed: String = "ghbdtn".chars().map(|c| m.map_char(c)).collect();
        assert_eq!(typed, "привет");
    }

    #[test]
    fn us_layout_keeps_latin() {
        let m = LayoutCharMapper::new(vec!["us".into(), "ru".into()], 0);
        assert_eq!(m.map_char('g'), 'g');
        assert_eq!(m.map_char('h'), 'h');
    }

    #[test]
    fn ru_layout_maps_uppercase() {
        let m = LayoutCharMapper::new(vec!["us".into(), "ru".into()], 1);
        assert_eq!(m.map_char('G'), 'П'); // g→п, G→П
    }

    #[test]
    fn unknown_layout_is_passthrough() {
        let m = LayoutCharMapper::new(vec!["us".into(), "ua".into()], 1);
        assert_eq!(m.map_char('g'), 'g');
    }

    #[test]
    fn set_current_switches_mapping() {
        let m = LayoutCharMapper::new(vec!["us".into(), "ru".into()], 0);
        assert_eq!(m.map_char('p'), 'p');
        m.set_current(1);
        assert_eq!(m.map_char('p'), 'з');
    }
}
