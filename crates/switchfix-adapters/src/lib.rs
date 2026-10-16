//! OS adapters implementing SwitchFix ports, selected by the platform layer.

pub mod platform;

pub use platform::portable::config_store::{ensure_config, TomlConfigStore};
pub use platform::portable::{
    NullSuggestionPresenter, SharedSuggestionPresenter, SharedTypingBuffer, SuggestionState,
    ZspellDictionary,
};
pub use platform::{
    create_platform, create_platform_from, detect, detect_from, detect_os, parse_desktop,
    parse_session, BuildOptions, DesktopKind, OsKind, Platform, PlatformError, PlatformInfo,
    PlatformServices, SessionKind,
};
