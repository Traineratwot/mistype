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
        (cfg.hotkeys.toggle_enabled.clone(), HotkeyEvent::ToggleEnabled),
        (cfg.hotkeys.fix_spelling.clone(), HotkeyEvent::FixSpelling),
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

/// Dynamic accept chords used while the suggestion bar is visible.
///
/// Bound via `hotkeys.accept_suggestion_1..=3` (max 3 slots). Matched only
/// while `bar_visible` is set so apps keep their normal shortcuts otherwise.
pub(crate) fn dynamic_suggestion_bindings(cfg: &AppConfig) -> Vec<(HotkeyBinding, HotkeyEvent)> {
    let mut out = Vec::new();
    let slots = [
        (cfg.hotkeys.accept_suggestion_1.as_str(), 1u8),
        (cfg.hotkeys.accept_suggestion_2.as_str(), 2),
        (cfg.hotkeys.accept_suggestion_3.as_str(), 3),
    ];
    for (s, n) in slots {
        if s.trim().is_empty() {
            continue;
        }
        match HotkeyBinding::parse(s) {
            Some(b) => out.push((b, HotkeyEvent::AcceptSuggestion(n))),
            None => warn!("invalid accept hotkey {n}: {s:?}"),
        }
    }
    if let Some(b) = HotkeyBinding::parse("ctrl+alt+0") {
        out.push((b, HotkeyEvent::AddToDictionary));
    }
    if let Some(b) = HotkeyBinding::parse("ctrl+alt+period") {
        out.push((b, HotkeyEvent::DismissSuggestions));
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
        cfg.hotkeys.toggle_enabled = "ctrl+pause".into();
        cfg.hotkeys.fix_spelling = "ctrl+f2".into();
        let parsed = parse_hotkey_bindings(&cfg);
        assert_eq!(parsed.len(), 3);
    }

    #[test]
    fn dynamic_suggestion_bindings_cover_slots() {
        let cfg = AppConfig::default();
        let parsed = dynamic_suggestion_bindings(&cfg);
        // accept 1..3 + learn + dismiss
        assert_eq!(parsed.len(), 5);
        assert!(parsed
            .iter()
            .any(|(b, e)| b.key == "1" && b.ctrl && b.alt
                && matches!(e, HotkeyEvent::AcceptSuggestion(1))));
    }

    #[test]
    fn dynamic_suggestion_bindings_respect_custom_and_empty() {
        let mut cfg = AppConfig::default();
        cfg.hotkeys.accept_suggestion_1 = "ctrl+f1".into();
        cfg.hotkeys.accept_suggestion_2 = String::new();
        cfg.hotkeys.accept_suggestion_3 = "ctrl+f3".into();
        let parsed = dynamic_suggestion_bindings(&cfg);
        assert!(parsed
            .iter()
            .any(|(b, e)| b.key == "f1" && matches!(e, HotkeyEvent::AcceptSuggestion(1))));
        assert!(parsed
            .iter()
            .any(|(b, e)| b.key == "f3" && matches!(e, HotkeyEvent::AcceptSuggestion(3))));
        assert!(!parsed
            .iter()
            .any(|(_, e)| matches!(e, HotkeyEvent::AcceptSuggestion(2))));
    }
}
