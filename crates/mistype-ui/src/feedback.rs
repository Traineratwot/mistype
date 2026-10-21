//! Hotkey feedback: tray flash duration constant.
//! Sound playback lives in `mistype_adapters::SystemSound` (SoundPort).

use std::time::Duration;

/// How long the tray icon stays lit after a hotkey.
pub const FLASH_DURATION: Duration = Duration::from_millis(350);
