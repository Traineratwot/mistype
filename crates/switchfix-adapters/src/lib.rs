//! OS adapters implementing SwitchFix ports.

pub mod clipboard;
pub mod config_store;
pub mod evdev_hotkey;
pub mod kde_layout;
pub mod uinput_injector;

pub use clipboard::WaylandClipboard;
pub use config_store::TomlConfigStore;
pub use evdev_hotkey::EvdevHotkeySource;
pub use kde_layout::KdeLayoutSwitcher;
pub use uinput_injector::UinputInjector;
