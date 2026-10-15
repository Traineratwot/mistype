//! Application configuration (serde model; persistence lives in adapters).

use serde::{Deserialize, Serialize};

fn def_true() -> bool {
    true
}
fn def_false() -> bool {
    false
}
fn def_copy_settle() -> u64 {
    400
}
fn def_paste_settle() -> u64 {
    100
}
fn def_buf_max() -> usize {
    256
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub hotkeys: Hotkeys,
    #[serde(default)]
    pub layouts: LayoutsCfg,
    #[serde(default)]
    pub behavior: Behavior,
    #[serde(default)]
    pub privacy: Privacy,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: General::default(),
            hotkeys: Hotkeys::default(),
            layouts: LayoutsCfg::default(),
            behavior: Behavior::default(),
            privacy: Privacy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct General {
    #[serde(default = "def_true")]
    pub enabled_on_start: bool,
    /// UI language: "ru" | "en"
    #[serde(default = "default_language")]
    pub language: String,
    /// "color" | "mono"
    #[serde(default = "default_icon_style")]
    pub tray_icon_style: String,
}

fn default_language() -> String {
    "ru".into()
}
fn default_icon_style() -> String {
    "color".into()
}

impl Default for General {
    fn default() -> Self {
        Self {
            enabled_on_start: true,
            language: default_language(),
            tray_icon_style: default_icon_style(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hotkeys {
    #[serde(default = "hk_sel")]
    pub convert_selection: String,
    #[serde(default = "hk_word")]
    pub convert_last_word: String,
    #[serde(default = "hk_term")]
    pub convert_terminal: String,
    #[serde(default = "hk_toggle")]
    pub toggle_enabled: String,
    #[serde(default = "def_false")]
    pub use_portal_hotkeys: bool,
    /// Fully consume hotkeys (EVIOCGRAB): the bound key does not reach apps.
    /// Non-hotkey keys are re-injected via uinput so typing still works.
    #[serde(default = "def_true")]
    pub hotkey_consume: bool,
}

fn hk_sel() -> String {
    "ctrl+f1".into()
}
fn hk_word() -> String {
    "ctrl+f2".into()
}
fn hk_term() -> String {
    "ctrl+shift+f1".into()
}
fn hk_toggle() -> String {
    "ctrl+pause".into()
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            convert_selection: hk_sel(),
            convert_last_word: hk_word(),
            convert_terminal: hk_term(),
            toggle_enabled: hk_toggle(),
            use_portal_hotkeys: false,
            hotkey_consume: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LayoutsCfg {
    /// Active layout short ids in KDE order, e.g. ["us", "ru"].
    #[serde(default = "default_active")]
    pub active: Vec<String>,
}

fn default_active() -> Vec<String> {
    vec!["us".into(), "ru".into()]
}

impl Default for LayoutsCfg {
    fn default() -> Self {
        Self {
            active: default_active(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Behavior {
    /// "auto" | "selection" | "word"
    #[serde(default = "def_capture_mode")]
    pub capture_mode: String,
    #[serde(default = "def_true")]
    pub fallback_to_word_on_no_selection: bool,
    /// "auto" | "retype" | "paste"
    #[serde(default = "def_replace_strategy")]
    pub replace_strategy: String,
    #[serde(default = "def_true")]
    pub restore_clipboard: bool,
    #[serde(default = "def_copy_settle")]
    pub copy_settle_ms: u64,
    #[serde(default = "def_paste_settle")]
    pub paste_settle_ms: u64,
    #[serde(default = "def_term_paste")]
    pub terminal_paste_combo: String,
    /// "line" | "backspaces"
    #[serde(default = "def_term_erase")]
    pub terminal_erase: String,
    #[serde(default)]
    pub excluded_apps: Vec<String>,
}

fn def_capture_mode() -> String {
    "auto".into()
}
fn def_replace_strategy() -> String {
    "auto".into()
}
fn def_term_paste() -> String {
    "ctrl+shift+v".into()
}
fn def_term_erase() -> String {
    "line".into()
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            capture_mode: def_capture_mode(),
            fallback_to_word_on_no_selection: true,
            replace_strategy: def_replace_strategy(),
            restore_clipboard: true,
            copy_settle_ms: def_copy_settle(),
            paste_settle_ms: def_paste_settle(),
            terminal_paste_combo: def_term_paste(),
            terminal_erase: def_term_erase(),
            excluded_apps: vec!["keepassxc".into(), "kwalletmanager5".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Privacy {
    #[serde(default = "def_true")]
    pub typing_buffer: bool,
    #[serde(default = "def_buf_max")]
    pub typing_buffer_max_keys: usize,
    #[serde(default = "def_false")]
    pub log_text_content: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            typing_buffer: true,
            typing_buffer_max_keys: def_buf_max(),
            log_text_content: false,
        }
    }
}

/// Why a config value was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigInvalid {
    Hotkey { field: String, value: String },
    LayoutIndex { value: String },
    SettleMs { field: String, value: u64 },
    EmptyLayouts,
}

impl std::fmt::Display for ConfigInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigInvalid::Hotkey { field, value } => {
                write!(f, "invalid hotkey '{value}' for {field}")
            }
            ConfigInvalid::LayoutIndex { value } => {
                write!(f, "invalid layout id '{value}' (expected short name like us/ru)")
            }
            ConfigInvalid::SettleMs { field, value } => {
                write!(f, "invalid {field}={value} (must be 0..=10000)")
            }
            ConfigInvalid::EmptyLayouts => write!(f, "layouts.active must not be empty"),
        }
    }
}

impl AppConfig {
    /// Validate user-facing fields (hotkeys, layout ids, settle delays).
    pub fn validate(&self) -> Result<(), Vec<ConfigInvalid>> {
        let mut errs = Vec::new();

        for (field, value) in [
            ("hotkeys.convert_selection", &self.hotkeys.convert_selection),
            ("hotkeys.convert_last_word", &self.hotkeys.convert_last_word),
            ("hotkeys.convert_terminal", &self.hotkeys.convert_terminal),
            ("hotkeys.toggle_enabled", &self.hotkeys.toggle_enabled),
        ] {
            if !is_valid_hotkey(value) {
                errs.push(ConfigInvalid::Hotkey {
                    field: field.to_string(),
                    value: value.clone(),
                });
            }
        }

        if self.layouts.active.is_empty() {
            errs.push(ConfigInvalid::EmptyLayouts);
        }
        for id in &self.layouts.active {
            if !is_valid_layout_id(id) {
                errs.push(ConfigInvalid::LayoutIndex { value: id.clone() });
            }
        }

        for (field, value) in [
            ("behavior.copy_settle_ms", self.behavior.copy_settle_ms),
            ("behavior.paste_settle_ms", self.behavior.paste_settle_ms),
        ] {
            if value > 10_000 {
                errs.push(ConfigInvalid::SettleMs {
                    field: field.to_string(),
                    value,
                });
            }
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }
}

/// Accepts `ctrl+f1`, `ctrl+shift+f1`, `ctrl+pause`, `alt+space`, …
fn is_valid_hotkey(s: &str) -> bool {
    let s = s.trim().to_ascii_lowercase();
    if s.is_empty() {
        return false;
    }
    let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    if key.is_empty() {
        return false;
    }
    // Key: single char, or a known key name.
    let key_ok = key.chars().count() == 1
        || matches!(
            key,
            "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11"
                | "f12" | "pause" | "break" | "space" | "tab" | "enter" | "return" | "esc"
                | "escape" | "backspace" | "delete" | "insert" | "home" | "end" | "pageup"
                | "pagedown" | "up" | "down" | "left" | "right"
        );
    if !key_ok {
        return false;
    }
    parts.iter().all(|m| {
        matches!(
            *m,
            "ctrl" | "control" | "shift" | "alt" | "meta" | "super" | "mod"
        )
    })
}

/// KDE-style short layout names: `us`, `ru`, `ua`, `de`, `us(intl)` …
fn is_valid_layout_id(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 32 {
        return false;
    }
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '(' | ')' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert!(AppConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_bad_hotkey() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.convert_selection = "".into();
        assert!(cfg.validate().is_err());
        cfg.hotkeys.convert_selection = "ctrl+".into();
        assert!(cfg.validate().is_err());
        cfg.hotkeys.convert_selection = "f1".into();
        assert!(cfg.validate().is_ok());
        cfg.hotkeys.convert_selection = "ctrl+shift+f1".into();
        assert!(cfg.validate().is_ok());
        cfg.hotkeys.convert_selection = "hyper+ctrl+f1".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_bad_layout_id() {
        let mut cfg = AppConfig::default();
        cfg.layouts.active = vec!["us".into(), "ru".into()];
        assert!(cfg.validate().is_ok());
        cfg.layouts.active = vec!["us".into(), "".into()];
        assert!(cfg.validate().is_err());
        cfg.layouts.active = vec!["us".into(), "1ru".into()];
        assert!(cfg.validate().is_err());
        cfg.layouts.active = vec!["us".into(), "us(intl)".into()];
        assert!(cfg.validate().is_ok());
        cfg.layouts.active = vec![];
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_huge_settle() {
        let mut cfg = AppConfig::default();
        cfg.behavior.copy_settle_ms = 60_000;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn parse_hotkey_examples() {
        for ok in ["ctrl+f1", "CTRL+F2", "ctrl+shift+f1", "ctrl+pause", "alt+space"] {
            assert!(is_valid_hotkey(ok), "{ok}");
        }
        for bad in ["", "+", "ctrl+", "ctrl+shift+", "ctrl+shift+ctrl", "foobar"] {
            assert!(!is_valid_hotkey(bad), "{bad}");
        }
    }
}
