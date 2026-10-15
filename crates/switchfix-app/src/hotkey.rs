//! Hotkey binding model: parse `ctrl+shift+f1` style strings and match key events.

/// Parsed hotkey: modifiers + main key name (canonical, lowercase).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    /// Canonical key name: `f1`, `insert`, `home`, `a`, …
    pub key: String,
}

impl HotkeyBinding {
    /// Parse a config-style combo. Returns `None` if invalid.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() {
            return None;
        }
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let key = parts.pop()?.to_string();
        if key.is_empty() {
            return None;
        }

        let mut b = Self {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            key,
        };
        for m in parts {
            match m {
                "ctrl" | "control" => b.ctrl = true,
                "alt" => b.alt = true,
                "shift" => b.shift = true,
                "meta" | "super" | "mod" => b.meta = true,
                _ => return None,
            }
        }
        // A bare modifier is not a binding.
        if matches!(
            b.key.as_str(),
            "ctrl" | "control" | "shift" | "alt" | "meta" | "super"
        ) {
            return None;
        }
        Some(b)
    }

    /// Match against live modifier state and a main key name.
    pub fn matches(&self, ctrl: bool, alt: bool, shift: bool, meta: bool, key: &str) -> bool {
        self.ctrl == ctrl
            && self.alt == alt
            && self.shift == shift
            && self.meta == meta
            && self.key == key
    }
}

/// Canonical key name → nothing here; OS-specific maps live in adapters.
/// This module only owns the combo grammar.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let b = HotkeyBinding::parse("ctrl+f1").unwrap();
        assert!(b.ctrl && !b.shift && b.key == "f1");
    }

    #[test]
    fn parse_insert() {
        let b = HotkeyBinding::parse("ctrl+insert").unwrap();
        assert!(b.ctrl && b.key == "insert");
    }

    #[test]
    fn parse_home() {
        let b = HotkeyBinding::parse("home").unwrap();
        assert!(!b.ctrl && b.key == "home");
    }

    #[test]
    fn parse_multi_mod() {
        let b = HotkeyBinding::parse("CTRL+Shift+F1").unwrap();
        assert!(b.ctrl && b.shift && b.key == "f1");
    }

    #[test]
    fn reject_modifier_only() {
        assert!(HotkeyBinding::parse("ctrl+").is_none());
        assert!(HotkeyBinding::parse("ctrl+shift").is_none());
        assert!(HotkeyBinding::parse("").is_none());
    }

    #[test]
    fn matches_exact_mods() {
        let b = HotkeyBinding::parse("ctrl+home").unwrap();
        assert!(b.matches(true, false, false, false, "home"));
        assert!(!b.matches(true, false, true, false, "home"));
        assert!(!b.matches(false, false, false, false, "home"));
        assert!(!b.matches(true, false, false, false, "end"));
    }
}
