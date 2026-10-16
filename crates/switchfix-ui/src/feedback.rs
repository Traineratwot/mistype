//! Hotkey feedback: system bell / desktop sound (best-effort, no extra deps).

use std::process::Command;
use std::time::Duration;

use tracing::{debug, warn};

/// How long the tray icon stays lit after a hotkey.
pub const FLASH_DURATION: Duration = Duration::from_millis(350);

/// Play a short confirmation sound. Never fails the caller — if no player is
/// available we just log and continue (tray flash is the reliable feedback).
pub fn play_hotkey_sound() {
    play_sound("message");
}

/// Short signal after a word is corrected (spell-fix / auto-fix / accept).
pub fn play_fix_sound() {
    play_sound("complete");
}

fn play_sound(event_id: &str) {
    // Prefer the freedesktop sound theme (present on KDE/Plasma).
    let candidates: &[(&str, &[&str])] = &[
        (
            "canberra-gtk-play",
            &["-i", event_id, "-d", "switchfix-spell"],
        ),
        (
            "canberra-gtk-play",
            &["-i", "message", "-d", "switchfix-spell"],
        ),
        (
            "paplay",
            &["/usr/share/sounds/freedesktop/stereo/bell.oga"],
        ),
        (
            "paplay",
            &["/usr/share/sounds/freedesktop/stereo/message.oga"],
        ),
        (
            "pw-play",
            &["/usr/share/sounds/freedesktop/stereo/bell.oga"],
        ),
    ];

    for (bin, args) in candidates {
        match Command::new(bin).args(*args).spawn() {
            Ok(mut child) => {
                // Reap in background so we don't leave zombies.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                debug!("sound via {bin} ({event_id})");
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                debug!("sound player {bin} failed: {e}");
                continue;
            }
        }
    }

    warn!("no system sound player found (canberra-gtk-play / paplay); silent feedback");
}
