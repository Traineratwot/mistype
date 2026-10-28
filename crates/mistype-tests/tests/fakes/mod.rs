//! Shared fakes for integration tests (PLAN.md §5).
//!
//! Every fake records a per-component log plus a shared cross-component log
//! so tests can assert the *order* of operations across ports
//! (save → put(marker) → chord → get → convert → put → chord → restore → set).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mistype_app::ports::*;
use mistype_domain::{KeyChord, LayoutId, LayoutIndex, LayoutInfo, WordSpan};
use tokio::sync::broadcast;

#[derive(Default)]
pub struct Log {
    pub entries: Mutex<Vec<String>>,
}

impl Log {
    pub fn push(&self, s: impl Into<String>) {
        self.entries.lock().unwrap().push(s.into());
    }

    pub fn all(&self) -> Vec<String> {
        self.entries.lock().unwrap().clone()
    }

    /// Assert that `a` appears before `b` in the log.
    pub fn assert_order(&self, a: &str, b: &str) {
        let log = self.all();
        let ia = log
            .iter()
            .position(|e| e.contains(a))
            .unwrap_or_else(|| panic!("{a:?} not found in {log:?}"));
        let ib = log
            .iter()
            .position(|e| e.contains(b))
            .unwrap_or_else(|| panic!("{b:?} not found in {log:?}"));
        assert!(ia < ib, "expected {a:?} before {b:?} in {log:?}");
    }

    pub fn contains(&self, s: &str) -> bool {
        self.all().iter().any(|e| e.contains(s))
    }
}

/// Cross-component event log used for order assertions spanning fakes.
pub type SharedLog = Arc<Log>;

/// Push an event into a component log and the shared log.
fn emit(component: &Log, shared: Option<&SharedLog>, event: impl Into<String>) {
    let event = event.into();
    if let Some(s) = shared {
        s.push(event.clone());
    }
    component.push(event);
}

#[derive(Default)]
pub struct FakeInjector {
    pub log: Log,
    pub shared: Option<SharedLog>,
}

#[async_trait]
impl TextInjector for FakeInjector {
    async fn backspace(&self, n: usize) -> Result<(), InjectError> {
        emit(&self.log, self.shared.as_ref(), format!("backspace:{n}"));
        Ok(())
    }
    async fn type_text(&self, text: &str, layout: &LayoutId) -> Result<(), InjectError> {
        emit(
            &self.log,
            self.shared.as_ref(),
            format!("type:{text}@{}", layout.as_str()),
        );
        Ok(())
    }
    async fn chord(&self, combo: KeyChord) -> Result<(), InjectError> {
        emit(&self.log, self.shared.as_ref(), format!("chord:{combo:?}"));
        Ok(())
    }
}

pub struct FakeClipboard {
    pub log: Log,
    pub text: Mutex<Option<String>>,
    pub fail_restore: bool,
    pub shared: Option<SharedLog>,
}

impl Default for FakeClipboard {
    fn default() -> Self {
        Self {
            log: Log::default(),
            text: Mutex::new(None),
            fail_restore: false,
            shared: None,
        }
    }
}

#[async_trait]
impl ClipboardPort for FakeClipboard {
    async fn save(&self) -> Result<SavedClipboard, ClipboardError> {
        emit(&self.log, self.shared.as_ref(), "save");
        Ok(SavedClipboard {
            text: self.text.lock().unwrap().clone(),
        })
    }
    async fn put_text(&self, t: &str) -> Result<(), ClipboardError> {
        emit(&self.log, self.shared.as_ref(), format!("put:{t}"));
        *self.text.lock().unwrap() = Some(t.to_string());
        Ok(())
    }
    async fn get_text(&self) -> Result<String, ClipboardError> {
        emit(&self.log, self.shared.as_ref(), "get");
        Ok(self.text.lock().unwrap().clone().unwrap_or_default())
    }
    async fn restore(&self, s: SavedClipboard) -> Result<(), ClipboardError> {
        emit(&self.log, self.shared.as_ref(), "restore");
        if self.fail_restore {
            return Err(ClipboardError::Failed("restore rejected".into()));
        }
        *self.text.lock().unwrap() = s.text;
        Ok(())
    }
}

/// Marker-protocol selection capture.
pub struct FakeSelectionCapture {
    pub log: Log,
    /// When `None`, the marker survives (no selection).
    pub selected: Option<String>,
    /// Clipboard snapshot reported to the caller (what restore() will bring back).
    pub saved_text: Option<String>,
    pub fail: bool,
    pub shared: Option<SharedLog>,
}

impl FakeSelectionCapture {
    pub fn no_selection() -> Self {
        Self {
            log: Log::default(),
            selected: None,
            saved_text: None,
            fail: false,
            shared: None,
        }
    }
}

#[async_trait]
impl SelectionCapture for FakeSelectionCapture {
    async fn capture_selection(
        &self,
        _target: CaptureTarget,
    ) -> Result<Option<CapturedSelection>, CaptureError> {
        emit(&self.log, self.shared.as_ref(), "capture_selection");
        if self.fail {
            return Err(CaptureError::Failed("capture failed".into()));
        }
        match &self.selected {
            Some(text) => Ok(Some(CapturedSelection {
                text: text.clone(),
                saved: SavedClipboard {
                    text: self.saved_text.clone(),
                },
            })),
            None => Ok(None),
        }
    }
}

#[derive(Default)]
pub struct FakeBuffer {
    pub log: Log,
    pub word: Mutex<Option<String>>,
    pub shared: Option<SharedLog>,
}

#[async_trait]
impl TypingBufferPort for FakeBuffer {
    fn take_last_word(&self) -> Option<WordSpan> {
        emit(&self.log, self.shared.as_ref(), "take_last_word");
        let t = self.word.lock().unwrap().take()?;
        let n = t.chars().count();
        Some(WordSpan {
            text: t,
            char_len: n,
        })
    }
    fn clear(&self) {
        emit(&self.log, self.shared.as_ref(), "buffer_clear");
        *self.word.lock().unwrap() = None;
    }
    fn is_enabled(&self) -> bool {
        true
    }
    fn push_char(&self, _c: char) {}
    fn backspace(&self) {}
}

pub struct FakeLayouts {
    pub log: Log,
    pub fail: bool,
    pub current: Mutex<LayoutIndex>,
    pub shared: Option<SharedLog>,
}

impl Default for FakeLayouts {
    fn default() -> Self {
        Self {
            log: Log::default(),
            fail: false,
            current: Mutex::new(LayoutIndex(0)),
            shared: None,
        }
    }
}

#[async_trait]
impl LayoutSwitcher for FakeLayouts {
    async fn current(&self) -> Result<LayoutIndex, LayoutError> {
        Ok(*self.current.lock().unwrap())
    }
    async fn set(&self, index: LayoutIndex) -> Result<(), LayoutError> {
        emit(
            &self.log,
            self.shared.as_ref(),
            format!("set_layout:{}", index.as_u32()),
        );
        if self.fail {
            return Err(LayoutError::Failed("KDE refused".into()));
        }
        *self.current.lock().unwrap() = index;
        Ok(())
    }
    async fn layouts(&self) -> Result<Vec<LayoutInfo>, LayoutError> {
        emit(&self.log, self.shared.as_ref(), "layouts");
        Ok(vec![
            LayoutInfo {
                index: LayoutIndex(0),
                id: LayoutId::new("us"),
                name: "English (US)".into(),
            },
            LayoutInfo {
                index: LayoutIndex(1),
                id: LayoutId::new("ru"),
                name: "Russian".into(),
            },
        ])
    }
    fn subscribe(&self) -> broadcast::Receiver<LayoutIndex> {
        let (tx, rx) = broadcast::channel(4);
        drop(tx);
        rx
    }
}

#[derive(Default)]
pub struct FakeNotifier {
    pub log: Log,
    pub shared: Option<SharedLog>,
}

#[async_trait]
impl Notifier for FakeNotifier {
    async fn notify(&self, msg: &str, level: NotifyLevel) -> Result<(), NotifyError> {
        emit(
            &self.log,
            self.shared.as_ref(),
            format!("notify:{level:?}:{msg}"),
        );
        Ok(())
    }
}

pub struct HarnessOptions {
    pub selected: Option<String>,
    pub word: Option<String>,
    pub clipboard_text: Option<String>,
    /// Clipboard snapshot returned by selection capture (defaults to `clipboard_text`).
    pub saved_text: Option<String>,
    pub layout_fail: bool,
    pub clipboard_restore_fail: bool,
    pub enabled: bool,
    pub restore_clipboard: bool,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            selected: None,
            word: None,
            clipboard_text: None,
            saved_text: None,
            layout_fail: false,
            clipboard_restore_fail: false,
            enabled: true,
            restore_clipboard: true,
        }
    }
}

/// Arc-shared fakes + CorrectionService under test.
pub struct Harness {
    pub service: mistype_app::CorrectionService,
    pub injector: Arc<FakeInjector>,
    pub clipboard: Arc<FakeClipboard>,
    pub selection: Arc<FakeSelectionCapture>,
    pub buffer: Arc<FakeBuffer>,
    pub layouts: Arc<FakeLayouts>,
    pub notifier: Arc<FakeNotifier>,
    /// Cross-component order log.
    pub events: SharedLog,
}

// `selection` / `layouts` are kept for future per-port assertions.
#[allow(dead_code)]
impl Harness {
    pub fn selection(&self) -> &FakeSelectionCapture {
        &self.selection
    }
    pub fn layouts(&self) -> &FakeLayouts {
        &self.layouts
    }
}

pub fn harness(opts: HarnessOptions) -> Harness {
    let events: SharedLog = Arc::new(Log::default());

    let injector = Arc::new(FakeInjector {
        log: Log::default(),
        shared: Some(events.clone()),
    });
    let clipboard = Arc::new(FakeClipboard {
        fail_restore: opts.clipboard_restore_fail,
        text: Mutex::new(opts.clipboard_text.clone()),
        shared: Some(events.clone()),
        ..Default::default()
    });
    let selection = Arc::new(FakeSelectionCapture {
        selected: opts.selected.clone(),
        saved_text: opts.saved_text.clone().or_else(|| opts.clipboard_text.clone()),
        shared: Some(events.clone()),
        ..FakeSelectionCapture::no_selection()
    });
    let buffer = Arc::new(FakeBuffer {
        word: Mutex::new(opts.word.clone()),
        shared: Some(events.clone()),
        ..Default::default()
    });
    let layouts = Arc::new(FakeLayouts {
        fail: opts.layout_fail,
        shared: Some(events.clone()),
        ..Default::default()
    });
    let notifier = Arc::new(FakeNotifier {
        log: Log::default(),
        shared: Some(events.clone()),
    });

    let mut cfg = mistype_app::AppConfig::default();
    cfg.behavior.restore_clipboard = opts.restore_clipboard;
    let (_tx, rx) = tokio::sync::watch::channel(cfg);

    let service = mistype_app::CorrectionService::new(
        injector.clone(),
        clipboard.clone(),
        selection.clone(),
        buffer.clone(),
        layouts.clone(),
        notifier.clone(),
        mistype_domain::ConversionSet::en_ru(),
        rx,
        opts.enabled,
    );

    Harness {
        service,
        injector,
        clipboard,
        selection,
        buffer,
        layouts,
        notifier,
        events,
    }
}
