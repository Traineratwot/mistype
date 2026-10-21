//! UI crate: tray (ksni) + settings (iced).

pub mod feedback;
pub mod settings_app;
pub mod tray;

pub use feedback::FLASH_DURATION;
pub use settings_app::run_settings;
pub use tray::{TrayHandle, TrayModel};
