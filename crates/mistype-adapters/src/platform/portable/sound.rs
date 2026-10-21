//! System sound feedback for hotkeys and text fixes (best-effort).

use std::process::Command;

use mistype_app::ports::SoundPort;
use tracing::{debug, warn};

pub struct SystemSound;

impl SystemSound {
    pub fn new() -> Self {
        Self
    }

    fn play_sound(event_id: &str) {
        // Prefer the freedesktop sound theme (present on KDE/Plasma).
        let candidates: &[(&str, &[&str])] = &[
            (
                "canberra-gtk-play",
                &["-i", event_id, "-d", "mistype"],
            ),
            (
                "canberra-gtk-play",
                &["-i", "message", "-d", "mistype"],
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
}

impl Default for SystemSound {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundPort for SystemSound {
    fn play_hotkey(&self) {
        Self::play_sound("message");
    }

    fn play_fix(&self) {
        Self::play_sound("complete");
    }
}
