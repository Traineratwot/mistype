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
