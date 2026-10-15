//! In-memory hotkey source for tests / fallback when the OS source is unavailable.

use std::sync::Mutex;

use switchfix_app::ports::{HotkeyEvent, HotkeySource};
use tokio::sync::mpsc;

/// In-memory hotkey source for tests / fallback when the OS source is unavailable.
pub struct ManualHotkeySource {
    tx: mpsc::Sender<HotkeyEvent>,
    rx: Mutex<Option<mpsc::Receiver<HotkeyEvent>>>,
}

impl ManualHotkeySource {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(16);
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }

    pub fn send(&self, ev: HotkeyEvent) {
        let _ = self.tx.try_send(ev);
    }
}

impl Default for ManualHotkeySource {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeySource for ManualHotkeySource {
    fn events(&self) -> mpsc::Receiver<HotkeyEvent> {
        self.rx.lock().unwrap().take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::channel(1);
            rx
        })
    }
}
