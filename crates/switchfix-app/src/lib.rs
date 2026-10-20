//! Application layer: use-cases and ports (traits) for SwitchFix.

pub mod config;
pub mod hotkey;
pub mod ports;
pub mod services;

pub use config::{AppConfig, ConfigInvalid, Llm};
pub use hotkey::HotkeyBinding;
pub use ports::*;
pub use services::CorrectionService;
