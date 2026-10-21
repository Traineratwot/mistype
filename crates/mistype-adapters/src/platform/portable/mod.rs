//! Portable (OS-agnostic) adapters shared by every platform module.

pub mod clipboard;
pub mod config_store;
pub mod heartbeat;
pub mod layout_chars;
pub mod llm_history;
pub mod log_notifier;
pub mod maps;
pub mod manual_hotkeys;
pub mod openai_corrector;
pub mod sound;
pub mod typing_buffer;
pub mod update_check;

pub use clipboard::ArboardClipboard;
pub use config_store::{ensure_config, TomlConfigStore};
pub use heartbeat::{
    clear_daemon_heartbeat, daemon_heartbeat_path, read_daemon_heartbeat, write_daemon_heartbeat,
    write_daemon_heartbeat_ex, DaemonHeartbeat, FileHeartbeat,
};
pub use layout_chars::LayoutCharMapper;
pub use llm_history::{
    append_llm_history, clear_llm_history, llm_history_path, llm_status_is_fresh,
    read_llm_history, read_llm_status, touch_llm_status, write_llm_busy_flag, write_llm_status,
    FileLlmHistory, LlmHistoryEntry, LlmStatus, FRESH_MS,
};
pub use log_notifier::LogNotifier;
pub use maps::{load_conversion_set, seed_default_maps, user_maps_dir};
pub use manual_hotkeys::ManualHotkeySource;
pub use openai_corrector::{OpenAiCorrector, OpenAiCorrectorFactory};
pub use sound::SystemSound;
pub use typing_buffer::SharedTypingBuffer;
pub use update_check::GithubUpdateChecker;
