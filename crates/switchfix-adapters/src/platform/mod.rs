//! Platform abstraction: detect OS/desktop, build port implementations.
//!
//! One [`Platform`] per OS/desktop stack. Selection:
//! compile-time `cfg(target_os)` for the OS, runtime `XDG_CURRENT_DESKTOP`
//! for the desktop environment (KDE / GNOME / other).

pub mod detect;
pub mod factory;
pub mod portable;
pub mod stub;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use std::sync::Arc;

use switchfix_app::config::AppConfig;
use switchfix_app::ports::{
    ClipboardPort, ConfigStore, FocusMonitor, HotkeySource, LayoutSwitcher, Notifier,
    SelectionCapture, TextInjector, TypingBufferPort,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    Linux,
    Windows,
    Macos,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopKind {
    Kde,
    Gnome,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Wayland,
    X11,
    Unknown,
}

/// What we detected on this machine. `id` is a stable string like `"linux/kde/wayland"`.
#[derive(Debug, Clone)]
pub struct PlatformInfo {
    pub os: OsKind,
    pub desktop: DesktopKind,
    pub session: SessionKind,
    pub id: String,
    /// true when a full implementation exists (today: linux+kde only).
    pub supported: bool,
    /// human-readable note; `Some` when `supported == false`.
    pub stub_reason: Option<String>,
}

/// Feature switches for `build_services` (maps 1:1 from daemon WireOptions).
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub allow_evdev: bool,
    pub allow_uinput: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            allow_evdev: true,
            allow_uinput: true,
        }
    }
}

/// All SwitchFix ports, built for the selected platform.
pub struct PlatformServices {
    pub injector: Arc<dyn TextInjector>,
    pub clipboard: Arc<dyn ClipboardPort>,
    pub selection: Arc<dyn SelectionCapture>,
    pub buffer: Arc<dyn TypingBufferPort>,
    pub layouts: Arc<dyn LayoutSwitcher>,
    pub hotkeys: Box<dyn HotkeySource>,
    pub config: Arc<dyn ConfigStore>,
    pub notifier: Arc<dyn Notifier>,
    /// Optional: focused-window monitor (KDE). `None` = no focus filtering.
    pub focus: Option<Arc<dyn FocusMonitor>>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("unsupported platform: {reason}")]
    Unsupported { reason: String },
    #[error("input injection unavailable: {0}")]
    Injection(String),
    #[error("layout switcher unavailable: {0}")]
    Layout(String),
    #[error("hotkeys unavailable: {0}")]
    Hotkeys(String),
    #[error("clipboard unavailable: {0}")]
    Clipboard(String),
    #[error("config store unavailable: {0}")]
    Config(String),
    #[error("platform error: {0}")]
    Other(String),
}

/// One OS/desktop stack. Implementations: `KdePlatform` (full), others stubs.
#[async_trait::async_trait]
pub trait Platform: Send + Sync {
    fn info(&self) -> PlatformInfo;

    /// Build all eight ports. Stubs return [`PlatformError::Unsupported`].
    async fn build_services(
        &self,
        cfg: &AppConfig,
        opts: &BuildOptions,
    ) -> Result<PlatformServices, PlatformError>;
}

pub use detect::{detect, detect_from, detect_os, parse_desktop, parse_session};
pub use factory::{create_platform, create_platform_from};
