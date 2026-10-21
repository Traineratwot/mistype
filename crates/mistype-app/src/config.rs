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
fn def_empty() -> String {
    String::new()
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
    /// External OpenAI-compatible LLM for «LLM-fix selection».
    #[serde(default)]
    pub llm: Llm,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: General::default(),
            hotkeys: Hotkeys::default(),
            layouts: LayoutsCfg::default(),
            behavior: Behavior::default(),
            privacy: Privacy::default(),
            llm: Llm::default(),
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
}

fn default_language() -> String {
    "ru".into()
}

impl Default for General {
    fn default() -> Self {
        Self {
            enabled_on_start: true,
            language: default_language(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hotkeys {
    #[serde(default = "hk_sel")]
    pub convert_selection: String,
    #[serde(default = "hk_toggle")]
    pub toggle_enabled: String,
    /// Send the whole selection to the external LLM and replace it.
    /// Empty string = action disabled.
    #[serde(default = "hk_llm_fix_selection")]
    pub llm_fix_selection: String,
    #[serde(default = "def_false")]
    pub use_portal_hotkeys: bool,
    /// Fully consume hotkeys (EVIOCGRAB): the bound key does not reach apps.
    /// Non-hotkey keys are re-injected via uinput so typing still works.
    #[serde(default = "def_true")]
    pub hotkey_consume: bool,
}

fn hk_sel() -> String {
    // Ctrl+Alt+R — convert (replace) wrong-layout text.
    "ctrl+alt+r".into()
}
fn hk_toggle() -> String {
    "ctrl+alt+t".into()
}
fn hk_llm_fix_selection() -> String {
    "alt+r".into()
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            convert_selection: hk_sel(),
            toggle_enabled: hk_toggle(),
            llm_fix_selection: hk_llm_fix_selection(),
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
    #[serde(default = "def_true")]
    pub fallback_to_word_on_no_selection: bool,
    #[serde(default = "def_true")]
    pub restore_clipboard: bool,
    #[serde(default = "def_copy_settle")]
    pub copy_settle_ms: u64,
    #[serde(default = "def_paste_settle")]
    pub paste_settle_ms: u64,
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
            fallback_to_word_on_no_selection: true,
            restore_clipboard: true,
            copy_settle_ms: def_copy_settle(),
            paste_settle_ms: def_paste_settle(),
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
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            typing_buffer: true,
            typing_buffer_max_keys: def_buf_max(),
        }
    }
}

/// External OpenAI-compatible chat-completions backend for «LLM-fix selection».
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Llm {
    /// Master switch for the hotkey feature.
    #[serde(default = "def_true")]
    pub enabled: bool,
    /// API base URL, e.g. `https://api.openai.com` or `http://127.0.0.1:11434`.
    /// Client appends `/v1/chat/completions`. Empty = feature off.
    #[serde(default = "def_empty")]
    pub base_url: String,
    /// Bearer token. Empty = omit Authorization header (local servers).
    #[serde(default = "def_empty")]
    pub api_token: String,
    /// System / initial prompt prepended as the first `system` message.
    #[serde(default = "default_llm_system_prompt")]
    pub system_prompt: String,
    /// Optional model id (`gpt-4o-mini`, `qwen2.5:0.5b`, …).
    /// Empty = omit the `model` field.
    #[serde(default = "def_empty")]
    pub model: String,
    /// HTTP timeout for the completion call, milliseconds.
    #[serde(default = "def_llm_timeout")]
    pub timeout_ms: u64,
}

/// Default system prompt for the LLM-fix feature (also used by the UI «Сбросить» button).
pub fn default_llm_system_prompt() -> String {
    "Исправь орфографию, грамматику и пунктуацию в тексте. \
     Верни ТОЛЬКО исправленный текст без пояснений и без кавычек."
        .into()
}
fn def_llm_timeout() -> u64 {
    15_000
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: def_empty(),
            api_token: def_empty(),
            system_prompt: default_llm_system_prompt(),
            model: def_empty(),
            timeout_ms: def_llm_timeout(),
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
    LlmBaseUrl { value: String },
    LlmTimeout { value: u64 },
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
            ConfigInvalid::LlmBaseUrl { value } => {
                write!(
                    f,
                    "invalid llm.base_url '{value}' (expected http:// or https:// URL)"
                )
            }
            ConfigInvalid::LlmTimeout { value } => {
                write!(f, "invalid llm.timeout_ms {value} (must be 500..=120000)")
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

        // LLM (external OpenAI-compatible API).
        let base = self.llm.base_url.trim();
        if !base.is_empty() && !(base.starts_with("http://") || base.starts_with("https://")) {
            errs.push(ConfigInvalid::LlmBaseUrl {
                value: self.llm.base_url.clone(),
            });
        }
        if !(500..=120_000).contains(&self.llm.timeout_ms) {
            errs.push(ConfigInvalid::LlmTimeout {
                value: self.llm.timeout_ms,
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

    #[test]
    fn llm_defaults_are_valid() {
        let cfg = AppConfig::default();
        assert!(cfg.llm.enabled);
        assert!(cfg.llm.base_url.is_empty());
        assert_eq!(cfg.llm.timeout_ms, 15_000);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_bad_llm_base_url() {
        let mut cfg = AppConfig::default();
        cfg.llm.base_url = "ftp://x".into();
        assert!(cfg.validate().is_err());
        cfg.llm.base_url = "https://api.openai.com".into();
        assert!(cfg.validate().is_ok());
        cfg.llm.base_url = "http://127.0.0.1:11434".into();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn rejects_bad_llm_timeout() {
        let mut cfg = AppConfig::default();
        cfg.llm.timeout_ms = 10;
        assert!(cfg.validate().is_err());
        cfg.llm.timeout_ms = 200_000;
        assert!(cfg.validate().is_err());
        cfg.llm.timeout_ms = 15_000;
        assert!(cfg.validate().is_ok());
    }
}
