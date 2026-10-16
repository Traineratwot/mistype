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
/// `Ctrl+1..=suggest_count` accept a slot, `Ctrl+0` adds to the dictionary,
/// `Ctrl+period` dismisses the bar. These are NOT config bindings — they are
/// only matched while `bar_visible` is set so apps keep their Ctrl+1 otherwise.
pub(crate) fn dynamic_suggestion_bindings(suggest_count: usize) -> Vec<(HotkeyBinding, HotkeyEvent)> {
    let mut out = Vec::new();
    let n = suggest_count.clamp(1, 5);
    for i in 1..=n {
        let key = i.to_string();
        if let Some(b) = HotkeyBinding::parse(&format!("ctrl+{key}")) {
            out.push((b, HotkeyEvent::AcceptSuggestion(i as u8)));
        }
    }
    if let Some(b) = HotkeyBinding::parse("ctrl+0") {
        out.push((b, HotkeyEvent::AddToDictionary));
    }
    if let Some(b) = HotkeyBinding::parse("ctrl+period") {
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
        let parsed = dynamic_suggestion_bindings(3);
        // ctrl+1, ctrl+2, ctrl+3, ctrl+0, ctrl+period
        assert_eq!(parsed.len(), 5);
    }
}
