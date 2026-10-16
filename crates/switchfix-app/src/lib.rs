//! Application layer: use-cases and ports (traits) for SwitchFix.

pub mod config;
pub mod spell_status;
pub mod hotkey;
pub mod ports;
pub mod services;

pub use config::{AppConfig, ConfigInvalid};
pub use hotkey::HotkeyBinding;
pub use ports::*;
pub use services::{CorrectionService, SpellAction, SpellService};

pub use spell_status::{read_spell_status, spell_history_path, spell_status_path, HistoryEntry, SpellStatus, SpellStatusStore};
