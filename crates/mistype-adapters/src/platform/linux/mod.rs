//! Linux platform modules (evdev, uinput) + per-DE selection.

pub mod evdev_hotkey;
pub mod gnome;
pub mod kde;
pub mod selftest;
pub mod uinput_injector;

pub use kde::probe_layouts;
pub use selftest::{run_hotkey_selftest, SelfTestReport};

use tracing::warn;

use crate::platform::{DesktopKind, Platform, PlatformInfo};
use mistype_app::config::AppConfig;
use mistype_app::hotkey::HotkeyBinding;
use mistype_app::ports::HotkeyEvent;

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
        (cfg.hotkeys.toggle_enabled.clone(), HotkeyEvent::ToggleEnabled),
        (
            cfg.hotkeys.llm_fix_selection.clone(),
            HotkeyEvent::LlmFixSelection,
        ),
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
    use mistype_app::config::AppConfig;

    #[test]
    fn parse_hotkey_bindings_valid_and_invalid() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.convert_selection = "ctrl+f1".into();
        cfg.hotkeys.toggle_enabled = "ctrl+pause".into();
        cfg.hotkeys.llm_fix_selection = "alt+r".into();
        let parsed = parse_hotkey_bindings(&cfg);
        assert_eq!(parsed.len(), 3);
        assert!(parsed
            .iter()
            .any(|(b, e)| b.key == "r" && b.alt && matches!(e, HotkeyEvent::LlmFixSelection)));
    }

    #[test]
    fn parse_hotkey_bindings_empty_skipped() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.convert_selection = String::new();
        let parsed = parse_hotkey_bindings(&cfg);
        assert!(!parsed
            .iter()
            .any(|(_, e)| matches!(e, HotkeyEvent::ConvertSelection)));
    }
}
