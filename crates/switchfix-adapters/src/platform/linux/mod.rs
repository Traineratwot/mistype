//! Linux platform modules (evdev, uinput) + per-DE selection.

pub mod evdev_hotkey;
pub mod gnome;
pub mod kde;
pub mod selftest;
pub mod uinput_injector;

use tracing::warn;

use crate::platform::{DesktopKind, Platform, PlatformInfo};
use switchfix_app::config::AppConfig;
use switchfix_app::hotkey::HotkeyBinding;
use switchfix_app::ports::HotkeyEvent;

/// Runtime DE selection inside Linux.
pub fn select(info: PlatformInfo) -> Box<dyn Platform> {
    match info.desktop {
        DesktopKind::Kde => Box::new(kde::KdePlatform::new(info)),
        DesktopKind::Gnome => Box::new(gnome::GnomePlatform::new(info)),
        _ => Box::new(crate::platform::stub::UnsupportedPlatform::new(info)),
    }
}

/// Parse `cfg.hotkeys.*` into (binding, event) pairs. Invalid strings are skipped.
pub(crate) fn parse_hotkey_bindings(
    cfg: &AppConfig,
) -> Vec<(HotkeyBinding, HotkeyEvent)> {
    let pairs = [
        (
            cfg.hotkeys.convert_selection.clone(),
            HotkeyEvent::ConvertSelection,
        ),
        (
            cfg.hotkeys.convert_last_word.clone(),
            HotkeyEvent::ConvertLastWord,
        ),
        (
            cfg.hotkeys.convert_terminal.clone(),
            HotkeyEvent::ConvertTerminal,
        ),
        (cfg.hotkeys.toggle_enabled.clone(), HotkeyEvent::ToggleEnabled),
    ];
    let mut out = Vec::new();
    for (s, ev) in pairs {
        if s.trim().is_empty() {
            continue; // disabled via Esc reset in settings
        }
        match HotkeyBinding::parse(&s) {
            Some(b) => out.push((b, ev)),
            None => warn!("invalid hotkey in config: {s:?}"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use switchfix_app::config::AppConfig;

    #[test]
    fn parse_hotkey_bindings_valid_and_invalid() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.convert_selection = "ctrl+f1".into();
        cfg.hotkeys.convert_last_word = "insert".into();
        cfg.hotkeys.convert_terminal = "ctrl+".into();
        cfg.hotkeys.toggle_enabled = "ctrl+pause".into();
        let parsed = parse_hotkey_bindings(&cfg);
        assert_eq!(parsed.len(), 3, "invalid string should be skipped");
    }
}
