//! Shared suggestion-bar state: visibility + last model.
//!
//! Implements `SuggestionPresenter` so the spell service can push hints here;
//! the floating bar UI process reads the same JSON file. The evdev hotkey
//! matcher watches `bar_visible` so accept chords (`Ctrl+1..N`) are only
//! intercepted while the bar is actually on screen.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use switchfix_app::ports::{NotifyError, SuggestionPresenter, SuggestionUi};
use tokio::sync::watch;
use tracing::debug;

/// Where the bar process reads the current hints from.
pub fn suggest_state_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.suggest.json")
}

/// Where the bar process writes click commands (`accept:N` / `add` / `dismiss`).
pub fn suggest_cmd_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.suggest.cmd")
}

/// Shared suggestion-bar state (portable).
pub struct SuggestionState {
    visible: Arc<AtomicBool>,
    model: Mutex<Option<SuggestionUi>>,
    /// Notified on every show/hide so in-process listeners can redraw.
    tx: watch::Sender<bool>,
}

impl SuggestionState {
    pub fn new() -> Arc<Self> {
        let (tx, _) = watch::channel(false);
        Arc::new(Self {
            visible: Arc::new(AtomicBool::new(false)),
            model: Mutex::new(None),
            tx,
        })
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    pub fn current(&self) -> Option<SuggestionUi> {
        self.model.lock().unwrap().clone()
    }

    /// Subscribe to visibility changes (`true` = bar should appear).
    pub fn subscribe_visible(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }

    /// Flag for the evdev matcher (accept chords only while visible).
    pub fn visible_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.visible)
    }
}

/// Presenter that records hints, flips visibility, and syncs the IPC file
/// the external bar process reads.
pub struct SharedSuggestionPresenter {
    inner: Arc<SuggestionState>,
}

impl SharedSuggestionPresenter {
    pub fn new(inner: Arc<SuggestionState>) -> Self {
        Self { inner }
    }

    pub fn state(&self) -> Arc<SuggestionState> {
        Arc::clone(&self.inner)
    }

    pub fn visible_flag(&self) -> Arc<AtomicBool> {
        self.inner.visible_flag()
    }
}

#[async_trait]
impl SuggestionPresenter for SharedSuggestionPresenter {
    async fn show(&self, model: SuggestionUi) -> Result<(), NotifyError> {
        debug!(
            "suggestion bar show: {:?} -> {:?}",
            model.original,
            model
                .slots
                .iter()
                .map(|s| s.text.clone())
                .collect::<Vec<_>>()
        );
        if let Ok(json) = serde_json::to_string(&model) {
            let _ = std::fs::write(suggest_state_path(), json);
        }
        *self.inner.model.lock().unwrap() = Some(model);
        self.inner.visible.store(true, Ordering::SeqCst);
        let _ = self.inner.tx.send(true);
        Ok(())
    }

    async fn hide(&self) -> Result<(), NotifyError> {
        debug!("suggestion bar hide");
        let _ = std::fs::remove_file(suggest_state_path());
        *self.inner.model.lock().unwrap() = None;
        self.inner.visible.store(false, Ordering::SeqCst);
        let _ = self.inner.tx.send(false);
        Ok(())
    }

    fn is_visible(&self) -> bool {
        self.inner.is_visible()
    }
}

/// Presenter that does nothing (spell mode `hotkey`, or tests).
pub struct NullSuggestionPresenter;

#[async_trait]
impl SuggestionPresenter for NullSuggestionPresenter {
    async fn show(&self, _model: SuggestionUi) -> Result<(), NotifyError> {
        Ok(())
    }
    async fn hide(&self) -> Result<(), NotifyError> {
        Ok(())
    }
    fn is_visible(&self) -> bool {
        false
    }
}
