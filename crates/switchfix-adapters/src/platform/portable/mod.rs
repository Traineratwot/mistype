//! Portable (OS-agnostic) adapters shared by every platform module.

pub mod clipboard;
pub mod config_store;
pub mod context_correctors;
pub mod layout_chars;
pub mod log_notifier;
pub mod maps;
pub mod manual_hotkeys;
pub mod suggestion_state;
pub mod typing_buffer;
pub mod zspell_dict;

pub use clipboard::ArboardClipboard;
pub use config_store::{ensure_config, TomlConfigStore};
pub use context_correctors::{
    build_context_corrector, LanguageToolCorrector, ProcessCorrector,
};
pub use layout_chars::LayoutCharMapper;
pub use log_notifier::LogNotifier;
pub use maps::{load_conversion_set, seed_default_maps, user_maps_dir};
pub use manual_hotkeys::ManualHotkeySource;
pub use suggestion_state::{
    clear_daemon_heartbeat, daemon_heartbeat_path, read_daemon_heartbeat, suggest_cmd_path,
    suggest_state_path, write_daemon_heartbeat, DaemonHeartbeat, NullSuggestionPresenter,
    SharedSuggestionPresenter, SuggestionState,
};
pub use typing_buffer::SharedTypingBuffer;
pub use zspell_dict::ZspellDictionary;
