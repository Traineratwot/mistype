//! Portable (OS-agnostic) adapters shared by every platform module.

pub mod clipboard;
pub mod config_store;
pub mod log_notifier;
pub mod manual_hotkeys;
pub mod typing_buffer;

pub use clipboard::ArboardClipboard;
pub use config_store::{ensure_config, TomlConfigStore};
pub use log_notifier::LogNotifier;
pub use manual_hotkeys::ManualHotkeySource;
pub use typing_buffer::SharedTypingBuffer;
