//! UI crate: tray (ksni) + settings (iced) + hotkey feedback.

pub mod feedback;
pub mod settings_app;
pub mod tray;

pub use feedback::{play_fix_sound, play_hotkey_sound, FLASH_DURATION};
pub use settings_app::run_settings;
pub use tray::{TrayHandle, TrayModel};
