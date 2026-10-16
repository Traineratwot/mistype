//! OS adapters implementing SwitchFix ports, selected by the platform layer.

pub mod platform;

pub use platform::portable::config_store::{ensure_config, TomlConfigStore};
pub use platform::portable::{
    load_conversion_set, seed_default_maps, suggest_cmd_path, suggest_state_path, user_maps_dir,
    LayoutCharMapper, NullSuggestionPresenter, SharedSuggestionPresenter, SharedTypingBuffer,
    SuggestionState, ZspellDictionary,
};
pub use platform::{
    create_platform, create_platform_from, detect, detect_from, detect_os, parse_desktop,
    parse_session, BuildOptions, DesktopKind, OsKind, Platform, PlatformError, PlatformInfo,
    PlatformServices, SessionKind,
};
