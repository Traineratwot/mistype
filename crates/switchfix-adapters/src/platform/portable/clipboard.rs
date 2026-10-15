//! Clipboard via `arboard`, plus selection capture (marker protocol).

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use switchfix_app::ports::{
    CaptureError, CapturedSelection, ClipboardError, ClipboardPort, InjectError, SavedClipboard,
    SelectionCapture, TextInjector,
};
use switchfix_domain::KeyChord;
use tokio::sync::Mutex as AsyncMutex;
use tracing::debug;
use uuid::Uuid;

/// Clipboard adapter (arboard). Selection capture needs an injector for Ctrl+Insert.
pub struct ArboardClipboard {
    lock: AsyncMutex<()>,
    injector: Option<Box<dyn TextInjector>>,
    settle_ms: u64,
    last_save: Mutex<Option<SavedClipboard>>,
}

impl ArboardClipboard {
    pub fn new(injector: Option<Box<dyn TextInjector>>, settle_ms: u64) -> Self {
        Self {
            lock: AsyncMutex::new(()),
            injector,
            settle_ms,
            last_save: Mutex::new(None),
        }
    }

    fn get_text_sync() -> Result<String, ClipboardError> {
        let mut cb = arboard::Clipboard::new()
            .map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
        cb.get_text()
            .map_err(|e| ClipboardError::Failed(e.to_string()))
    }

    fn put_text_sync(text: &str) -> Result<(), ClipboardError> {
        let mut cb = arboard::Clipboard::new()
            .map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
        cb.set_text(text.to_string())
            .map_err(|e| ClipboardError::Failed(e.to_string()))
    }
}

#[async_trait]
impl ClipboardPort for ArboardClipboard {
    async fn save(&self) -> Result<SavedClipboard, ClipboardError> {
        let _g = self.lock.lock().await;
        let text = Self::get_text_sync().ok();
        let snap = SavedClipboard { text };
        *self.last_save.lock().unwrap() = Some(snap.clone());
        Ok(snap)
    }

    async fn put_text(&self, text: &str) -> Result<(), ClipboardError> {
        let _g = self.lock.lock().await;
        Self::put_text_sync(text)
    }

    async fn get_text(&self) -> Result<String, ClipboardError> {
        let _g = self.lock.lock().await;
        Self::get_text_sync()
    }

    async fn restore(&self, saved: SavedClipboard) -> Result<(), ClipboardError> {
        let _g = self.lock.lock().await;
        match saved.text {
            Some(t) => Self::put_text_sync(&t),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl SelectionCapture for ArboardClipboard {
    /// Clipboard-marker protocol:
    /// 1. save user clipboard
    /// 2. put unique marker
    /// 3. press Ctrl+Insert (copy selection)
    /// 4. read clipboard — if marker survived, there was no selection
    ///
    /// On success returns the selected text plus the saved clipboard snapshot
    /// so the caller can restore it after replacing the selection.
    async fn capture_selection(&self) -> Result<Option<CapturedSelection>, CaptureError> {
        let _g = self.lock.lock().await;

        let marker = format!("SWITCHFIX_MARK_{}", Uuid::new_v4());
        let saved = Self::get_text_sync().ok();
        let snap = SavedClipboard {
            text: saved.clone(),
        };
        *self.last_save.lock().unwrap() = Some(snap.clone());

        Self::put_text_sync(&marker).map_err(|e| CaptureError::Failed(e.to_string()))?;

        let Some(inj) = &self.injector else {
            return Err(CaptureError::Failed(
                "no injector for selection capture".into(),
            ));
        };

        inj.chord(KeyChord::CtrlInsert)
            .await
            .map_err(|e: InjectError| CaptureError::Failed(e.to_string()))?;

        tokio::time::sleep(Duration::from_millis(self.settle_ms)).await;

        let now = Self::get_text_sync().map_err(|e| CaptureError::Failed(e.to_string()))?;

        if now == marker {
            // No selection — restore clipboard immediately.
            if let Some(s) = &saved {
                let _ = Self::put_text_sync(s);
            }
            *self.last_save.lock().unwrap() = None;
            return Ok(None);
        }

        debug!("selection captured ({} chars)", now.chars().count());
        Ok(Some(CapturedSelection {
            text: now,
            saved: snap,
        }))
    }
}

impl ArboardClipboard {
    /// Restore the snapshot saved during `capture_selection`.
    pub async fn restore_last(&self) -> Result<(), ClipboardError> {
        let saved = self.last_save.lock().unwrap().take();
        if let Some(s) = saved {
            self.restore(s).await
        } else {
            Ok(())
        }
    }
}
