//! Ports (traits) — boundaries between application and OS adapters.

use async_trait::async_trait;
use switchfix_domain::{KeyChord, LayoutId, LayoutIndex, LayoutInfo, WordSpan};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};

// ---------- errors ----------

#[derive(Debug, Error)]
pub enum InjectError {
    #[error("uinput unavailable: {0}")]
    Device(String),
    #[error("unknown char for layout {layout}: {ch:?}")]
    UnknownChar { layout: String, ch: char },
    #[error("inject failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard unavailable: {0}")]
    Unavailable(String),
    #[error("clipboard timeout")]
    Timeout,
    #[error("clipboard failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("selection capture failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("dbus unavailable: {0}")]
    Dbus(String),
    #[error("layout switch rejected for index {0}")]
    Rejected(u32),
    #[error("layout error: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum HotkeyError {
    #[error("evdev unavailable: {0}")]
    Evdev(String),
    #[error("hotkey source failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config io: {0}")]
    Io(String),
    #[error("config parse: {0}")]
    Parse(String),
}

#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("notify failed: {0}")]
    Failed(String),
}

// ---------- payloads ----------

/// Text-only snapshot of the user clipboard (restore is best-effort).
#[derive(Debug, Clone)]
pub struct SavedClipboard {
    pub text: Option<String>,
}

/// Result of a successful selection capture (marker protocol).
///
/// Carries the user clipboard snapshot so the caller can restore it after paste.
#[derive(Debug, Clone)]
pub struct CapturedSelection {
    pub text: String,
    pub saved: SavedClipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyEvent {
    ConvertSelection,
    ToggleEnabled,
}

// ---------- ports ----------

#[async_trait]
pub trait TextInjector: Send + Sync {
    async fn backspace(&self, count: usize) -> Result<(), InjectError>;
    /// Type `text` as if the user pressed keys producing these chars in `layout`.
    async fn type_text(&self, text: &str, layout: &LayoutId) -> Result<(), InjectError>;
    async fn chord(&self, combo: KeyChord) -> Result<(), InjectError>;
}

#[async_trait]
pub trait ClipboardPort: Send + Sync {
    async fn save(&self) -> Result<SavedClipboard, ClipboardError>;
    async fn put_text(&self, text: &str) -> Result<(), ClipboardError>;
    async fn get_text(&self) -> Result<String, ClipboardError>;
    async fn restore(&self, saved: SavedClipboard) -> Result<(), ClipboardError>;
}

#[async_trait]
pub trait SelectionCapture: Send + Sync {
    /// Clipboard-marker protocol. `Ok(None)` = no selection (marker survived
    /// and the user clipboard has already been restored).
    async fn capture_selection(&self) -> Result<Option<CapturedSelection>, CaptureError>;
}

#[async_trait]
pub trait TypingBufferPort: Send + Sync {
    fn take_last_word(&self) -> Option<WordSpan>;
    fn clear(&self);
    fn is_enabled(&self) -> bool;
    fn push_char(&self, c: char);
    fn backspace(&self);
}

#[async_trait]
pub trait LayoutSwitcher: Send + Sync {
    async fn current(&self) -> Result<LayoutIndex, LayoutError>;
    async fn set(&self, index: LayoutIndex) -> Result<(), LayoutError>;
    async fn layouts(&self) -> Result<Vec<LayoutInfo>, LayoutError>;
    fn subscribe(&self) -> broadcast::Receiver<LayoutIndex>;
}

#[async_trait]
pub trait HotkeySource: Send + Sync {
    /// Stream of hotkey events. Source must not consume physical keys (pass-through).
    fn events(&self) -> mpsc::Receiver<HotkeyEvent>;
}

#[async_trait]
pub trait ConfigStore: Send + Sync {
    fn load(&self) -> Result<crate::config::AppConfig, ConfigError>;
    fn save(&self, cfg: &crate::config::AppConfig) -> Result<(), ConfigError>;
}

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, msg: &str, level: NotifyLevel) -> Result<(), NotifyError>;
}

/// Snapshot of the focused window, used to pick correction strategy
/// (terminal vs GUI) and to skip work in games / excluded apps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWindow {
    /// Process / app id (`konsole`, `firefox`, `steam`, …). Lowercase.
    pub app_id: String,
    pub title: String,
    pub fullscreen: bool,
}

#[async_trait]
pub trait FocusMonitor: Send + Sync {
    /// Best-effort; `None` when the compositor cannot tell us (then we
    /// fall back to the generic GUI strategy and never block).
    async fn active_window(&self) -> Option<ActiveWindow>;
}
