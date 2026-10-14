//! Application layer: use-cases and ports (traits) for SwitchFix.

pub mod config;
pub mod ports;
pub mod services;

pub use config::AppConfig;
pub use ports::*;
pub use services::CorrectionService;
