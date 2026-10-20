//! OS adapters implementing Mistype ports, selected by the platform layer.

pub mod platform;

pub use platform::portable::config_store::{ensure_config, TomlConfigStore};
pub use platform::portable::{
    append_llm_history, clear_daemon_heartbeat, clear_llm_history, daemon_heartbeat_path,
    llm_history_path, llm_status_is_fresh, load_conversion_set, read_daemon_heartbeat,
    read_llm_history, read_llm_status, seed_default_maps, touch_llm_status, user_maps_dir,
    write_daemon_heartbeat, write_daemon_heartbeat_ex, write_llm_busy_flag, write_llm_status,
    DaemonHeartbeat, LayoutCharMapper, LlmHistoryEntry, LlmStatus, OpenAiCorrector,
    SharedTypingBuffer, FRESH_MS,
};
pub use platform::{
    create_platform, create_platform_from, detect, detect_from, detect_os, parse_desktop,
    parse_session, BuildOptions, DesktopKind, OsKind, Platform, PlatformError, PlatformInfo,
    PlatformServices, SessionKind,
};
