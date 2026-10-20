//! OS adapters implementing SwitchFix ports, selected by the platform layer.

pub mod platform;

pub use platform::portable::config_store::{ensure_config, TomlConfigStore};
pub use platform::portable::{
    clear_daemon_heartbeat, daemon_heartbeat_path, load_conversion_set, read_daemon_heartbeat,
    seed_default_maps, user_maps_dir, write_daemon_heartbeat, DaemonHeartbeat, LayoutCharMapper,
    OpenAiCorrector, SharedTypingBuffer,
};
pub use platform::{
    create_platform, create_platform_from, detect, detect_from, detect_os, parse_desktop,
    parse_session, BuildOptions, DesktopKind, OsKind, Platform, PlatformError, PlatformInfo,
    PlatformServices, SessionKind,
};
