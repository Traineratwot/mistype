//! Ports (traits) — boundaries between application and OS adapters.

use std::sync::Arc;

use async_trait::async_trait;
use mistype_domain::{KeyChord, LayoutId, LayoutIndex, LayoutInfo, WordSpan};
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

/// Errors from the external text corrector (LLM API).
#[derive(Debug, Error)]
pub enum TextFixError {
    #[error("http: {0}")]
    Http(String),
    #[error("api: {0}")]
    Api(String),
    #[error("parse: {0}")]
    Parse(String),
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
    LlmFixSelection,
}

/// A word boundary / fragment update from the typing buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordEvent {
    /// A word was finished (separator typed).
    Completed {
        word: WordSpan,
        /// The separator character that ended the word (space, comma, …).
        separator: char,
    },
    /// Characters were added — current in-progress fragment.
    /// Also used to cancel a pending LLM undo (user typed something else).
    Updated(String),
    /// Physical Backspace (one character). Used to undo a recent LLM replace.
    Backspace,
}

// ---------- ports ----------

#[async_trait]
pub trait TextInjector: Send + Sync {
    async fn backspace(&self, count: usize) -> Result<(), InjectError>;
    /// Type `text` as if the user pressed keys producing these chars in `layout`.
    async fn type_text(&self, text: &str, layout: &LayoutId) -> Result<(), InjectError>;
    async fn chord(&self, combo: KeyChord) -> Result<(), InjectError>;
    /// Move the text cursor left `count` characters (no erase).
    /// Used so a word replace never eats the separator before the word.
    async fn cursor_left(&self, count: usize) -> Result<(), InjectError> {
        let _ = count;
        Ok(())
    }
    /// Move the text cursor right `count` characters (no erase).
    /// After a replace the cursor must end up *after* the trailing
    /// separator so the next typed key is not glued to the replacement.
    async fn cursor_right(&self, count: usize) -> Result<(), InjectError> {
        let _ = count;
        Ok(())
    }
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
    /// Current in-progress word (after the last separator), without consuming.
    fn peek_current_word(&self) -> Option<String> {
        None
    }
    /// Overwrite the buffer contents **without** emitting word events.
    /// Used after we inject a replacement so the next Space doesn't
    /// re-complete the old word and trigger a second replace.
    fn set_contents(&self, text: &str) {
        let _ = text;
    }
    /// Last up-to-`max_chars` characters (context for LT/LLM). Default empty.
    fn peek_tail(&self, max_chars: usize) -> String {
        let _ = max_chars;
        String::new()
    }
    /// Wait until no physical key has been pressed for `quiet_ms`
    /// (give up after `max_ms`). Default is a plain sleep.
    async fn wait_keys_quiet(&self, quiet_ms: u64, max_ms: u64) {
        let _ = max_ms;
        tokio::time::sleep(std::time::Duration::from_millis(quiet_ms)).await;
    }
    /// Stream of word-boundary / fragment events.
    fn word_events(&self) -> broadcast::Receiver<WordEvent> {
        // A never-used channel; adapters that care override this.
        let (tx, rx) = broadcast::channel(1);
        drop(tx);
        rx
    }
}

/// Whole-selection text fixer («LLM-fix selection»).
#[async_trait]
pub trait TextCorrector: Send + Sync {
    /// Human-readable id for logs (`openai`, …).
    fn name(&self) -> &'static str;
    /// Fix spelling/grammar of a whole text block.
    /// `Ok(original)` or empty string = no change.
    async fn correct_text(&self, text: &str) -> Result<String, TextFixError>;
}

/// Connection probe for the settings UI: list models + tiny test completion.
#[async_trait]
pub trait LlmProbe: Send + Sync {
    /// `GET {base}/v1/models` → model ids (empty list if none).
    async fn list_models(
        &self,
        base_url: &str,
        api_token: &str,
        timeout_ms: u64,
    ) -> Result<Vec<String>, TextFixError>;
    /// Tiny chat completion to prove the endpoint works.
    /// Returns a short reply snippet for the status line.
    async fn test_chat(&self, cfg: &crate::config::Llm) -> Result<String, TextFixError>;
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

// ---------- update check ----------

/// Result of a one-shot update check (GitHub Releases).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Already running the latest release; carries the current version.
    UpToDate(String),
    /// A newer release exists; carries the remote tag (e.g. `v2.1.0`).
    Available(String),
    /// Network/parse failure — informational only, never a product defect.
    Unavailable(String),
}

/// One-shot release check, used by `mistype doctor`.
/// Sync by design: doctor is a blocking CLI, no need for async HTTP.
pub trait UpdateChecker: Send + Sync {
    fn check(&self) -> UpdateStatus;
}

// ---------- clipboard read (settings UI paste) ----------

/// Read-only clipboard access for the settings UI «Вставить» buttons.
/// Separate from [`ClipboardPort`] so the marker protocol / `last_save`
/// state is never touched by a simple paste.
pub trait ClipboardRead: Send + Sync {
    fn get_text(&self) -> Result<String, ClipboardError>;
}

// ---------- sound ----------

/// Hotkey / fix feedback sounds. Best-effort: never fails the caller.
pub trait SoundPort: Send + Sync {
    /// Short confirmation after a hotkey is received.
    fn play_hotkey(&self);
    /// Short signal after a text replacement (LLM fix / undo).
    fn play_fix(&self);
}

// ---------- corrector factory ----------

/// Builds corrector / probe instances from `[llm]` config.
/// Keeps `OpenAiCorrector` construction behind a port so wiring does not
/// hardcode the concrete adapter.
pub trait CorrectorFactory: Send + Sync {
    /// `None` when the feature is off (disabled or empty `base_url`).
    fn build_corrector(&self, cfg: &crate::config::Llm) -> Option<Arc<dyn TextCorrector>>;
    /// Always returns a probe (for doctor / settings «Тест»).
    fn build_probe(&self, cfg: &crate::config::Llm) -> Arc<dyn LlmProbe>;
}

// ---------- LLM history / heartbeat ----------

/// One LLM-fix history entry (persisted for the Plasma widget).
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Rolling LLM-fix history for the Plasma widget.
pub trait LlmHistoryStore: Send + Sync {
    fn append(&self, entry: LlmHistoryEntry);
    fn clear(&self);
    fn read(&self) -> Vec<LlmHistoryEntry>;
}

/// Daemon liveness file (heartbeat).
pub trait HeartbeatPort: Send + Sync {
    fn touch(&self, enabled: bool, busy: bool);
    fn clear(&self);
}
