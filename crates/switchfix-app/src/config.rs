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
    #[serde(default)]
    pub spell: Spell,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: General::default(),
            hotkeys: Hotkeys::default(),
            layouts: LayoutsCfg::default(),
            behavior: Behavior::default(),
            privacy: Privacy::default(),
            spell: Spell::default(),
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
    #[serde(default = "hk_toggle")]
    pub toggle_enabled: String,
    #[serde(default = "hk_fix_spelling")]
    pub fix_spelling: String,
    /// Send the whole selection to LLM and replace it with the corrected text.
    /// Empty string = action disabled.
    #[serde(default = "hk_llm_fix_selection")]
    pub llm_fix_selection: String,
    /// Accept Nth suggestion while the bar is visible (max 3).
    #[serde(default = "hk_accept1")]
    pub accept_suggestion_1: String,
    #[serde(default = "hk_accept2")]
    pub accept_suggestion_2: String,
    #[serde(default = "hk_accept3")]
    pub accept_suggestion_3: String,
    #[serde(default = "def_false")]
    pub use_portal_hotkeys: bool,
    /// Fully consume hotkeys (EVIOCGRAB): the bound key does not reach apps.
    /// Non-hotkey keys are re-injected via uinput so typing still works.
    #[serde(default = "def_true")]
    pub hotkey_consume: bool,
}

fn hk_sel() -> String {
    // Ctrl+Alt+R — convert (replace) wrong-layout text. Avoids Ctrl+1 tab switching.
    "ctrl+alt+r".into()
}
fn hk_toggle() -> String {
    "ctrl+alt+t".into()
}
fn hk_fix_spelling() -> String {
    "ctrl+alt+f".into()
}
fn hk_llm_fix_selection() -> String {
    "alt+r".into()
}
fn hk_accept1() -> String {
    "ctrl+alt+1".into()
}
fn hk_accept2() -> String {
    "ctrl+alt+2".into()
}
fn hk_accept3() -> String {
    "ctrl+alt+3".into()
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            convert_selection: hk_sel(),
            toggle_enabled: hk_toggle(),
            fix_spelling: hk_fix_spelling(),
            llm_fix_selection: hk_llm_fix_selection(),
            accept_suggestion_1: hk_accept1(),
            accept_suggestion_2: hk_accept2(),
            accept_suggestion_3: hk_accept3(),
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
    /// App ids where corrections are forbidden (password managers, …).
    #[serde(default = "def_excluded_apps")]
    pub excluded_apps: Vec<String>,
    /// App ids that should use the terminal paste strategy
    /// (Ctrl+A / Ctrl+K / Ctrl+Shift+V) without a separate hotkey.
    #[serde(default = "def_terminal_apps")]
    pub terminal_apps: Vec<String>,
    /// Skip corrections when the focused window is fullscreen (games, video).
    #[serde(default = "def_true")]
    pub disable_in_fullscreen: bool,
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
fn def_excluded_apps() -> Vec<String> {
    vec!["keepassxc".into(), "kwalletmanager5".into()]
}
fn def_terminal_apps() -> Vec<String> {
    vec![
        "konsole".into(),
        "yakuake".into(),
        "xterm".into(),
        "alacritty".into(),
        "kitty".into(),
        "foot".into(),
        "gnome-terminal".into(),
        "xfce4-terminal".into(),
        "tilix".into(),
        "wezterm".into(),
    ]
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
            excluded_apps: def_excluded_apps(),
            terminal_apps: def_terminal_apps(),
            disable_in_fullscreen: true,
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

/// Spellcheck + suggestion bar settings (Phase 2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spell {
    #[serde(default = "def_true")]
    pub enabled: bool,
    /// "off" | "hint" | "hotkey" | "auto"
    #[serde(default = "def_spell_mode")]
    pub mode: String,
    /// Dictionary language ids to load, e.g. `["ru", "en"]`.
    #[serde(default = "def_spell_langs")]
    pub languages: Vec<String>,
    /// Skip words shorter than this.
    #[serde(default = "def_min_word_len")]
    pub min_word_len: usize,
    /// Start completions after this many typed characters.
    #[serde(default = "def_completion_min_chars")]
    pub completion_min_chars: usize,
    /// How many candidate slots the bar shows.
    #[serde(default = "def_suggest_count")]
    pub suggest_count: usize,
    /// "bottom" | "top"
    #[serde(default = "def_bar_position")]
    pub bar_position: String,
    #[serde(default = "def_bar_timeout")]
    pub bar_timeout_ms: u64,
    /// Minimum word length for silent auto-replace.
    #[serde(default = "def_auto_fix_min_len")]
    pub auto_fix_min_len: usize,
    /// Play a short sound when a word is replaced.
    #[serde(default = "def_true")]
    pub sound: bool,
    /// Spell backend: `dict` (compat only) | `hybrid` (dict + context) | `auto`.
    #[serde(default = "def_spell_backend")]
    pub backend: String,
    /// LanguageTool HTTP base URL (e.g. `http://127.0.0.1:8010`). Empty = off.
    #[serde(default = "def_empty")]
    pub lt_url: String,
    /// External corrector command: reads JSON on stdin, writes JSON on stdout.
    /// Empty = off. Example: `llm-spell` or `ollama run llama3.2:1b`.
    #[serde(default = "def_empty")]
    pub llm_cmd: String,
    /// Max recent characters sent as context to LT/LLM.
    #[serde(default = "def_context_chars")]
    pub context_chars: usize,
    /// Wait this long after a word (Space) before spell-checking, so the
    /// user can keep typing without the next word being torn by a replace.
    #[serde(default = "def_debounce_ms")]
    pub debounce_ms: u64,
    /// Master switch for the «LLM-fix selection» hotkey feature (not auto-replace).
    #[serde(default = "def_true")]
    pub llm_selection: bool,
}

fn def_spell_mode() -> String {
    "hint".into()
}
fn def_spell_langs() -> Vec<String> {
    vec!["ru".into(), "en".into()]
}
fn def_min_word_len() -> usize {
    3
}
fn def_completion_min_chars() -> usize {
    3
}
fn def_suggest_count() -> usize {
    3
}
fn def_bar_position() -> String {
    "bottom".into()
}
fn def_bar_timeout() -> u64 {
    4500
}
fn def_auto_fix_min_len() -> usize {
    4
}
fn def_spell_backend() -> String {
    "hybrid".into()
}
fn def_empty() -> String {
    String::new()
}
fn def_context_chars() -> usize {
    80
}
fn def_debounce_ms() -> u64 {
    180
}

impl Default for Spell {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: def_spell_mode(),
            languages: def_spell_langs(),
            min_word_len: def_min_word_len(),
            completion_min_chars: def_completion_min_chars(),
            suggest_count: def_suggest_count(),
            bar_position: def_bar_position(),
            bar_timeout_ms: def_bar_timeout(),
            auto_fix_min_len: def_auto_fix_min_len(),
            sound: true,
            backend: def_spell_backend(),
            lt_url: def_empty(),
            llm_cmd: def_empty(),
            context_chars: def_context_chars(),
            debounce_ms: def_debounce_ms(),
            llm_selection: true,
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
    SpellMode { value: String },
    SpellBarPosition { value: String },
    SpellSuggestCount { value: usize },
    SpellBackend { value: String },
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
            ConfigInvalid::SpellMode { value } => {
                write!(f, "invalid spell.mode '{value}' (expected off|hint|hotkey|auto)")
            }
            ConfigInvalid::SpellBarPosition { value } => {
                write!(f, "invalid spell.bar_position '{value}' (expected bottom|top)")
            }
            ConfigInvalid::SpellSuggestCount { value } => {
                write!(f, "invalid spell.suggest_count {value} (must be 1..=5)")
            }
            ConfigInvalid::SpellBackend { value } => {
                write!(
                    f,
                    "invalid spell.backend '{value}' (expected dict|hybrid|auto)"
                )
            }
        }
    }
}

impl AppConfig {
    /// Validate user-facing fields (hotkeys, layout ids, settle delays).
    pub fn validate(&self) -> Result<(), Vec<ConfigInvalid>> {
        let mut errs = Vec::new();

        for (field, value) in [
            ("hotkeys.convert_selection", &self.hotkeys.convert_selection),
            ("hotkeys.toggle_enabled", &self.hotkeys.toggle_enabled),
            ("hotkeys.fix_spelling", &self.hotkeys.fix_spelling),
            ("hotkeys.llm_fix_selection", &self.hotkeys.llm_fix_selection),
        ] {
            // Empty = action disabled (Esc reset in the settings UI).
            if value.trim().is_empty() {
                continue;
            }
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

        if !matches!(self.spell.mode.as_str(), "off" | "hint" | "hotkey" | "auto") {
            errs.push(ConfigInvalid::SpellMode {
                value: self.spell.mode.clone(),
            });
        }
        if !matches!(self.spell.bar_position.as_str(), "bottom" | "top") {
            errs.push(ConfigInvalid::SpellBarPosition {
                value: self.spell.bar_position.clone(),
            });
        }
        if !(1..=5).contains(&self.spell.suggest_count) {
            errs.push(ConfigInvalid::SpellSuggestCount {
                value: self.spell.suggest_count,
            });
        }
        if !matches!(self.spell.backend.as_str(), "dict" | "hybrid" | "auto") {
            errs.push(ConfigInvalid::SpellBackend {
                value: self.spell.backend.clone(),
            });
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
                | "f12" | "f13" | "f14" | "f15" | "f16" | "f17" | "f18" | "f19" | "f20"
                | "pause" | "break" | "space" | "tab" | "enter" | "return" | "esc"
                | "escape" | "backspace" | "delete" | "insert" | "home" | "end" | "pageup"
                | "pagedown" | "up" | "down" | "left" | "right" | "period" | "comma"
                | "capslock" | "caps" | "numlock" | "scrolllock" | "menu" | "printscreen"
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
        // Empty = disabled (Esc reset) — valid.
        cfg.hotkeys.convert_selection = "".into();
        assert!(cfg.validate().is_ok());
        cfg.hotkeys.convert_selection = "ctrl+".into();
        assert!(cfg.validate().is_err());
        cfg.hotkeys.convert_selection = "f1".into();
        assert!(cfg.validate().is_ok());
        cfg.hotkeys.convert_selection = "ctrl+shift+f1".into();
        assert!(cfg.validate().is_ok());
        cfg.hotkeys.convert_selection = "capslock".into();
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
