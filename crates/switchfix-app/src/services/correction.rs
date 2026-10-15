//! Main use-case: convert text typed in the wrong keyboard layout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use switchfix_domain::{
    convert, CorrectionOutcome, CorrectionRequest, CorrectionState, KeyChord,
    LayoutId, LayoutIndex, ConversionSet,
};
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::config::AppConfig;
use crate::ports::{
    CapturedSelection, ClipboardPort, LayoutSwitcher, NotifyLevel, Notifier, SavedClipboard,
    SelectionCapture, TextInjector, TypingBufferPort,
};

/// Orchestrates capture → convert → replace → switch-layout.
pub struct CorrectionService {
    injector: Arc<dyn TextInjector>,
    clipboard: Arc<dyn ClipboardPort>,
    selection: Arc<dyn SelectionCapture>,
    buffer: Arc<dyn TypingBufferPort>,
    layouts: Arc<dyn LayoutSwitcher>,
    notifier: Arc<dyn Notifier>,
    tables: ConversionSet,
    config: watch::Receiver<AppConfig>,
    enabled: AtomicBool,
    state_tx: watch::Sender<CorrectionState>,
}

impl CorrectionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        injector: Arc<dyn TextInjector>,
        clipboard: Arc<dyn ClipboardPort>,
        selection: Arc<dyn SelectionCapture>,
        buffer: Arc<dyn TypingBufferPort>,
        layouts: Arc<dyn LayoutSwitcher>,
        notifier: Arc<dyn Notifier>,
        tables: ConversionSet,
        config: watch::Receiver<AppConfig>,
        enabled: bool,
    ) -> Self {
        let (state_tx, _) = watch::channel(CorrectionState::Idle);
        Self {
            injector,
            clipboard,
            selection,
            buffer,
            layouts,
            notifier,
            tables,
            config,
            enabled: AtomicBool::new(enabled),
            state_tx,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn state(&self) -> CorrectionState {
        *self.state_tx.borrow()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<CorrectionState> {
        self.state_tx.subscribe()
    }

    pub async fn toggle_enabled(&self) -> bool {
        let now = !self.is_enabled();
        self.enabled.store(now, Ordering::SeqCst);
        if !now {
            self.buffer.clear();
        }
        let msg = if now {
            "SwitchFix: включён"
        } else {
            "SwitchFix: приостановлен"
        };
        let _ = self.notifier.notify(msg, NotifyLevel::Info).await;
        now
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        if !on {
            self.buffer.clear();
        }
    }

    pub async fn handle(&self, req: CorrectionRequest) -> CorrectionOutcome {
        if !self.is_enabled() {
            return CorrectionOutcome::Disabled;
        }

        let _ = self.state_tx.send(CorrectionState::Capturing);

        let outcome = match req {
            CorrectionRequest::ConvertSelection => self.convert_selection().await,
            CorrectionRequest::ConvertLastWord => self.convert_last_word().await,
            CorrectionRequest::ConvertTerminal => self.convert_terminal().await,
        };

        let next = match &outcome {
            CorrectionOutcome::Converted { .. } | CorrectionOutcome::Partial { .. } => {
                CorrectionState::Idle
            }
            CorrectionOutcome::NoInputFound | CorrectionOutcome::Disabled => CorrectionState::Idle,
            CorrectionOutcome::Failed(_) => CorrectionState::Error {
                recoverable: true,
            },
        };
        let _ = self.state_tx.send(next);
        outcome
    }

    // ----- internals -----

    fn cfg(&self) -> AppConfig {
        self.config.borrow().clone()
    }

    async fn convert_selection(&self) -> CorrectionOutcome {
        let cfg = self.cfg();

        let captured = match self.selection.capture_selection().await {
            Ok(Some(sel)) => sel,
            Ok(None) => {
                if cfg.behavior.fallback_to_word_on_no_selection {
                    return self.convert_last_word().await;
                }
                return CorrectionOutcome::NoInputFound;
            }
            Err(e) => {
                warn!("selection capture failed: {e}");
                return CorrectionOutcome::Failed(e.to_string());
            }
        };

        self.replace_via_clipboard(captured).await
    }

    async fn convert_last_word(&self) -> CorrectionOutcome {
        let Some(span) = self.buffer.take_last_word() else {
            return CorrectionOutcome::NoInputFound;
        };

        let converted = match convert(&span.text, &self.tables, None, None) {
            Ok(c) => c,
            Err(e) => {
                debug!("convert last word: {e}");
                return CorrectionOutcome::NoInputFound;
            }
        };

        let _ = self.state_tx.send(CorrectionState::Replacing);

        if let Err(e) = self.injector.backspace(span.char_len).await {
            return CorrectionOutcome::Failed(format!("backspace: {e}"));
        }

        let target_layout = LayoutId::new(converted.to.as_str().to_string());
        if let Err(e) = self.injector.type_text(&converted.converted, &target_layout).await {
            return CorrectionOutcome::Failed(format!("type_text: {e}"));
        }

        self.switch_layout_for(&converted.converted).await;
        self.buffer.clear();

        CorrectionOutcome::Converted {
            original: converted.original,
            converted: converted.converted,
            layout_switched_to: None,
        }
    }

    async fn convert_terminal(&self) -> CorrectionOutcome {
        // Terminal mode: use last word from buffer (or selection), then
        // clear the input line and paste via Ctrl+Shift+V.
        let cfg = self.cfg();
        let (text, saved) = match self.buffer.take_last_word() {
            Some(span) => (span.text, None),
            None => match self.selection.capture_selection().await {
                Ok(Some(sel)) => (sel.text, Some(sel.saved)),
                _ => return CorrectionOutcome::NoInputFound,
            },
        };

        let converted = match convert(&text, &self.tables, None, None) {
            Ok(c) => c,
            Err(e) => {
                debug!("convert terminal: {e}");
                if let Some(s) = saved {
                    self.restore_clipboard(s, &cfg).await;
                }
                return CorrectionOutcome::NoInputFound;
            }
        };

        let _ = self.state_tx.send(CorrectionState::Replacing);

        // Put result in clipboard first.
        if let Err(e) = self.clipboard.put_text(&converted.converted).await {
            self.restore_clipboard_optional(saved, &cfg).await;
            return CorrectionOutcome::Failed(format!("clipboard put: {e}"));
        }

        // Clear line: Ctrl+A, Ctrl+K  (or backspaces per config).
        if cfg.behavior.terminal_erase == "backspaces" {
            // text length unknown precisely in terminal — best-effort skip
        } else {
            let _ = self.injector.chord(KeyChord::CtrlA).await;
            let _ = self.injector.chord(KeyChord::CtrlK).await;
        }

        if let Err(e) = self.injector.chord(KeyChord::CtrlShiftV).await {
            self.restore_clipboard_optional(saved, &cfg).await;
            return CorrectionOutcome::Failed(format!("paste: {e}"));
        }

        tokio::time::sleep(Duration::from_millis(cfg.behavior.paste_settle_ms)).await;
        self.restore_clipboard_optional(saved, &cfg).await;

        self.switch_layout_for(&converted.converted).await;

        CorrectionOutcome::Converted {
            original: converted.original,
            converted: converted.converted,
            layout_switched_to: None,
        }
    }

    async fn replace_via_clipboard(&self, captured: CapturedSelection) -> CorrectionOutcome {
        let cfg = self.cfg();
        let CapturedSelection { text: selected, saved } = captured;

        let converted = match convert(&selected, &self.tables, None, None) {
            Ok(c) => c,
            Err(e) => {
                debug!("convert selection: {e}");
                // Nothing was pasted — put the user clipboard back.
                self.restore_clipboard(saved, &cfg).await;
                return CorrectionOutcome::NoInputFound;
            }
        };

        let _ = self.state_tx.send(CorrectionState::Replacing);

        if let Err(e) = self.clipboard.put_text(&converted.converted).await {
            self.restore_clipboard(saved, &cfg).await;
            return CorrectionOutcome::Failed(format!("clipboard put: {e}"));
        }

        if let Err(e) = self.injector.chord(KeyChord::CtrlV).await {
            self.restore_clipboard(saved, &cfg).await;
            return CorrectionOutcome::Failed(format!("paste: {e}"));
        }

        // Give the app time to consume paste before restore.
        tokio::time::sleep(Duration::from_millis(cfg.behavior.paste_settle_ms)).await;

        let _ = self.state_tx.send(CorrectionState::RestoringClipboard);
        self.restore_clipboard(saved, &cfg).await;

        let _ = self.state_tx.send(CorrectionState::SwitchingLayout);
        self.switch_layout_for(&converted.converted).await;

        CorrectionOutcome::Converted {
            original: converted.original,
            converted: converted.converted,
            layout_switched_to: None,
        }
    }

    /// Best-effort clipboard restore (never fails the conversion).
    async fn restore_clipboard(&self, saved: SavedClipboard, cfg: &AppConfig) {
        if !cfg.behavior.restore_clipboard {
            return;
        }
        if let Err(e) = self.clipboard.restore(saved).await {
            warn!("clipboard restore failed: {e}");
            let _ = self
                .notifier
                .notify("Не удалось восстановить буфер обмена", NotifyLevel::Warning)
                .await;
        }
    }

    async fn restore_clipboard_optional(&self, saved: Option<SavedClipboard>, cfg: &AppConfig) {
        if let Some(s) = saved {
            self.restore_clipboard(s, cfg).await;
        }
    }

    /// Best-effort: never fails the text conversion if layout switch fails.
    async fn switch_layout_for(&self, converted: &str) -> Option<LayoutIndex> {
        use switchfix_domain::detect_direction;

        // Target layout = "other" side after conversion.
        // After convert, text is in the *destination* layout's alphabet.
        let target_id = detect_direction(converted).map(|(detected, _other)| detected);
        let Some(target_id) = target_id else {
            return None;
        };

        let layouts = match self.layouts.layouts().await {
            Ok(l) => l,
            Err(e) => {
                warn!("layout list failed: {e}");
                let _ = self
                    .notifier
                    .notify(
                        "Текст исправлен, но раскладка не переключилась",
                        NotifyLevel::Warning,
                    )
                    .await;
                return None;
            }
        };

        let target = layouts.iter().find(|l| l.id == target_id);
        let Some(info) = target else {
            debug!("target layout {} not in KDE list", target_id);
            return None;
        };

        match self.layouts.set(info.index).await {
            Ok(()) => Some(info.index),
            Err(e) => {
                warn!("layout switch failed: {e}");
                let _ = self
                    .notifier
                    .notify(
                        "Текст исправлен, но раскладка не переключилась",
                        NotifyLevel::Warning,
                    )
                    .await;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::*;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use switchfix_domain::{LayoutInfo, WordSpan};
    use tokio::sync::broadcast;

    // ----- fakes -----

    #[derive(Default)]
    struct FakeInjector {
        log: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl TextInjector for FakeInjector {
        async fn backspace(&self, n: usize) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("bs:{n}"));
            Ok(())
        }
        async fn type_text(&self, text: &str, layout: &LayoutId) -> Result<(), InjectError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("type:{text}:{}", layout.as_str()));
            Ok(())
        }
        async fn chord(&self, combo: KeyChord) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("chord:{combo:?}"));
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeClipboard {
        text: Mutex<Option<String>>,
        log: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ClipboardPort for FakeClipboard {
        async fn save(&self) -> Result<SavedClipboard, ClipboardError> {
            Ok(SavedClipboard {
                text: self.text.lock().unwrap().clone(),
            })
        }
        async fn put_text(&self, t: &str) -> Result<(), ClipboardError> {
            self.log.lock().unwrap().push(format!("put:{t}"));
            *self.text.lock().unwrap() = Some(t.to_string());
            Ok(())
        }
        async fn get_text(&self) -> Result<String, ClipboardError> {
            Ok(self.text.lock().unwrap().clone().unwrap_or_default())
        }
        async fn restore(&self, s: SavedClipboard) -> Result<(), ClipboardError> {
            self.log.lock().unwrap().push("restore".into());
            *self.text.lock().unwrap() = s.text;
            Ok(())
        }
    }

    /// Marker protocol: if `text` is None → no selection (marker survives).
    struct FakeSelection {
        text: Option<String>,
    }

    #[async_trait]
    impl SelectionCapture for FakeSelection {
        async fn capture_selection(&self) -> Result<Option<CapturedSelection>, CaptureError> {
            Ok(self.text.clone().map(|text| CapturedSelection {
                text,
                saved: SavedClipboard { text: None },
            }))
        }
    }

    #[derive(Default)]
    struct FakeBuffer {
        word: Mutex<Option<String>>,
    }

    #[async_trait]
    impl TypingBufferPort for FakeBuffer {
        fn take_last_word(&self) -> Option<WordSpan> {
            let t = self.word.lock().unwrap().take()?;
            let n = t.chars().count();
            Some(WordSpan {
                text: t,
                char_len: n,
            })
        }
        fn clear(&self) {
            *self.word.lock().unwrap() = None;
        }
        fn is_enabled(&self) -> bool {
            true
        }
        fn push_char(&self, _c: char) {}
        fn backspace(&self) {}
    }

    struct FakeLayouts {
        fail: bool,
    }

    #[async_trait]
    impl LayoutSwitcher for FakeLayouts {
        async fn current(&self) -> Result<LayoutIndex, LayoutError> {
            Ok(LayoutIndex(0))
        }
        async fn set(&self, index: LayoutIndex) -> Result<(), LayoutError> {
            if self.fail {
                Err(LayoutError::Failed("boom".into()))
            } else {
                let _ = index;
                Ok(())
            }
        }
        async fn layouts(&self) -> Result<Vec<LayoutInfo>, LayoutError> {
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
    struct FakeNotifier {
        log: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Notifier for FakeNotifier {
        async fn notify(&self, msg: &str, _level: NotifyLevel) -> Result<(), NotifyError> {
            self.log.lock().unwrap().push(msg.to_string());
            Ok(())
        }
    }

    fn service(
        sel: Option<String>,
        word: Option<String>,
        layout_fail: bool,
        enabled: bool,
    ) -> (
        CorrectionService,
        Arc<FakeInjector>,
        Arc<FakeClipboard>,
        Arc<FakeBuffer>,
    ) {
        let injector = Arc::new(FakeInjector::default());
        let clipboard = Arc::new(FakeClipboard::default());
        let selection = Arc::new(FakeSelection { text: sel });
        let buffer = Arc::new(FakeBuffer::default());
        if let Some(w) = word {
            *buffer.word.lock().unwrap() = Some(w);
        }
        let layouts = Arc::new(FakeLayouts { fail: layout_fail });
        let notifier = Arc::new(FakeNotifier::default());
        let (_tx, rx) = watch::channel(AppConfig::default());
        let svc = CorrectionService::new(
            injector.clone(),
            clipboard.clone(),
            selection,
            buffer.clone(),
            layouts,
            notifier,
            ConversionSet::en_ru(),
            rx,
            enabled,
        );
        (svc, injector, clipboard, buffer)
    }

    #[tokio::test]
    async fn last_word_converts_ghbdtn() {
        let (svc, inj, _clip, buf) = service(None, Some("ghbdtn".into()), false, true);
        let out = svc.handle(CorrectionRequest::ConvertLastWord).await;
        match out {
            CorrectionOutcome::Converted {
                original,
                converted,
                ..
            } => {
                assert_eq!(original, "ghbdtn");
                assert_eq!(converted, "привет");
            }
            other => panic!("unexpected {other:?}"),
        }
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l == "bs:6"), "log={log:?}");
        assert!(log.iter().any(|l| l == "type:привет:ru"), "log={log:?}");
        assert!(buf.word.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn last_word_empty_is_no_input() {
        let (svc, _i, _c, _b) = service(None, None, false, true);
        let out = svc.handle(CorrectionRequest::ConvertLastWord).await;
        assert_eq!(out, CorrectionOutcome::NoInputFound);
    }

    #[tokio::test]
    async fn disabled_is_disabled() {
        let (svc, inj, _c, _b) = service(None, Some("ghbdtn".into()), false, false);
        let out = svc.handle(CorrectionRequest::ConvertLastWord).await;
        assert_eq!(out, CorrectionOutcome::Disabled);
        assert!(inj.log.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn selection_uses_clipboard_paste() {
        let (svc, inj, clip, _b) = service(Some("ghbdtn".into()), None, false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert!(out.is_success(), "{out:?}");
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l.contains("chord:CtrlV")), "log={log:?}");
        let clog = clip.log.lock().unwrap().clone();
        assert!(clog.iter().any(|l| l == "put:привет"), "clog={clog:?}");
        // Clipboard must be restored after paste (plan §6).
        assert!(clog.iter().any(|l| l == "restore"), "clog={clog:?}");
    }

    #[tokio::test]
    async fn selection_no_convert_restores_clipboard() {
        // "123" has no letters → convert fails → clipboard restored, no paste.
        let (svc, inj, clip, _b) = service(Some("123".into()), None, false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert_eq!(out, CorrectionOutcome::NoInputFound);
        let log = inj.log.lock().unwrap().clone();
        assert!(!log.iter().any(|l| l.contains("chord:CtrlV")), "log={log:?}");
        let clog = clip.log.lock().unwrap().clone();
        assert!(clog.iter().any(|l| l == "restore"), "clog={clog:?}");
    }

    #[tokio::test]
    async fn selection_falls_back_to_word() {
        let (svc, inj, _c, _b) = service(None, Some("руддщ".into()), false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert!(out.is_success(), "{out:?}");
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l == "bs:5"), "log={log:?}");
    }

    #[tokio::test]
    async fn layout_fail_still_reports_conversion() {
        let (svc, _i, _c, _b) = service(None, Some("ghbdtn".into()), true, true);
        let out = svc.handle(CorrectionRequest::ConvertLastWord).await;
        // text was replaced even if layout switch failed
        assert!(out.is_success(), "{out:?}");
    }

    #[tokio::test]
    async fn toggle_clears_buffer() {
        let (svc, _i, _c, buf) = service(None, Some("secret".into()), false, true);
        assert!(svc.is_enabled());
        let now = svc.toggle_enabled().await;
        assert!(!now);
        assert!(buf.word.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn terminal_chords() {
        let (svc, inj, _c, _b) = service(None, Some("ghbdtn".into()), false, true);
        let out = svc.handle(CorrectionRequest::ConvertTerminal).await;
        assert!(out.is_success(), "{out:?}");
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l.contains("CtrlA")), "log={log:?}");
        assert!(log.iter().any(|l| l.contains("CtrlK")), "log={log:?}");
        assert!(log.iter().any(|l| l.contains("CtrlShiftV")), "log={log:?}");
    }
}
