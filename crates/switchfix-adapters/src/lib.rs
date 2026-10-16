//! OS adapters implementing SwitchFix ports, selected by the platform layer.

pub mod platform;

pub use platform::portable::config_store::{ensure_config, TomlConfigStore};
pub use platform::portable::{
    clear_daemon_heartbeat, daemon_heartbeat_path, load_conversion_set, read_daemon_heartbeat,
    seed_default_maps, suggest_cmd_path, suggest_state_path, user_maps_dir, write_daemon_heartbeat,
    DaemonHeartbeat, LayoutCharMapper, NullSuggestionPresenter, SharedSuggestionPresenter,
    SharedTypingBuffer, SuggestionState, ZspellDictionary,
};
// Spell status / history live in switchfix-app (no adapters dependency).
pub use switchfix_app::{
    read_spell_status, spell_history_path, spell_status_path, HistoryEntry, SpellStatus,
    SpellStatusStore,
};
pub use platform::{
    create_platform, create_platform_from, detect, detect_from, detect_os, parse_desktop,
    parse_session, BuildOptions, DesktopKind, OsKind, Platform, PlatformError, PlatformInfo,
    PlatformServices, SessionKind,
};
