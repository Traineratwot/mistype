//! Main use-case: convert text typed in the wrong keyboard layout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use switchfix_domain::{
    convert, ConversionSet, CorrectionOutcome, CorrectionRequest, CorrectionState, KeyChord,
    LayoutId, LayoutIndex,
};
use tokio::sync::watch;
use tracing::{debug, warn};

use crate::config::AppConfig;
use crate::ports::{
    CapturedSelection, ClipboardPort, FocusMonitor, LayoutSwitcher, Notifier, NotifyLevel,
    SavedClipboard, SelectionCapture, TextCorrector, TextInjector, TypingBufferPort,
};

/// Undo target after an LLM selection replace (Backspace rolls it back).
struct UndoState {
    original: String,
    replacement: String,
    when: Instant,
}

/// How long Backspace still means "undo last LLM replace".
const UNDO_WINDOW: Duration = Duration::from_secs(4);

/// Orchestrates capture → convert → replace → switch-layout.
pub struct CorrectionService {
    injector: Arc<dyn TextInjector>,
    clipboard: Arc<dyn ClipboardPort>,
    selection: Arc<dyn SelectionCapture>,
    buffer: Arc<dyn TypingBufferPort>,
    layouts: Arc<dyn LayoutSwitcher>,
    notifier: Arc<dyn Notifier>,
    focus: Option<Arc<dyn FocusMonitor>>,
    /// External OpenAI-compatible corrector for «LLM-fix selection».
    text_corrector: std::sync::Mutex<Option<Arc<dyn TextCorrector>>>,
    /// Set after an LLM replace so the next Backspace undoes it.
    undo: std::sync::Mutex<Option<UndoState>>,
    /// Skip undo while WE inject backspaces / type_text.
    suppress: AtomicBool,
    tables: std::sync::RwLock<ConversionSet>,
    config: watch::Receiver<AppConfig>,
    enabled: AtomicBool,
    state_tx: watch::Sender<CorrectionState>,
}

impl CorrectionService {
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
        Self::with_focus(
            injector,
            clipboard,
            selection,
            buffer,
            layouts,
            notifier,
            None,
            tables,
            config,
            enabled,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_focus(
        injector: Arc<dyn TextInjector>,
        clipboard: Arc<dyn ClipboardPort>,
        selection: Arc<dyn SelectionCapture>,
        buffer: Arc<dyn TypingBufferPort>,
        layouts: Arc<dyn LayoutSwitcher>,
        notifier: Arc<dyn Notifier>,
        focus: Option<Arc<dyn FocusMonitor>>,
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
            focus,
            text_corrector: std::sync::Mutex::new(None),
            undo: std::sync::Mutex::new(None),
            suppress: AtomicBool::new(false),
            tables: std::sync::RwLock::new(tables),
            config,
            enabled: AtomicBool::new(enabled),
            state_tx,
        }
    }

    /// Install the external OpenAI-compatible corrector.
    pub fn set_text_corrector(&self, ctx: Arc<dyn TextCorrector>) {
        *self.text_corrector.lock().unwrap() = Some(ctx);
    }

    /// Swap conversion tables at runtime (config reload).
    pub fn replace_tables(&self, tables: ConversionSet) {
        *self.tables.write().unwrap() = tables;
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
            self.clear_undo();
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
            self.clear_undo();
        }
    }

    pub async fn handle(&self, req: CorrectionRequest) -> CorrectionOutcome {
        if !self.is_enabled() {
            return CorrectionOutcome::Disabled;
        }

        let cfg = self.cfg();
        let win = match &self.focus {
            Some(f) => f.active_window().await,
            None => None,
        };

        if let Some(w) = &win {
            let app = w.app_id.to_ascii_lowercase();
            if cfg
                .behavior
                .excluded_apps
                .iter()
                .any(|e| app == e.to_ascii_lowercase() || app.contains(&e.to_ascii_lowercase()))
            {
                debug!("skip: excluded app '{app}'");
                return CorrectionOutcome::Disabled;
            }
            if w.fullscreen && cfg.behavior.disable_in_fullscreen {
                debug!("skip: fullscreen window '{app}'");
                return CorrectionOutcome::Disabled;
            }
        }

        // Auto-pick terminal strategy when the focused app is a terminal —
        // no separate hotkey needed.
        let req = match (&req, &win) {
            (CorrectionRequest::ConvertSelection, Some(w)) => {
                let app = w.app_id.to_ascii_lowercase();
                if cfg
                    .behavior
                    .terminal_apps
                    .iter()
                    .any(|t| app == t.to_ascii_lowercase() || app.contains(&t.to_ascii_lowercase()))
                {
                    debug!("auto terminal strategy for '{app}'");
                    CorrectionRequest::ConvertTerminal
                } else {
                    req
                }
            }
            _ => req,
        };

        let _ = self.state_tx.send(CorrectionState::Capturing);

        let outcome = match req {
            CorrectionRequest::ConvertSelection => self.convert_selection().await,
            CorrectionRequest::ConvertTerminal => self.convert_terminal().await,
            CorrectionRequest::LlmFixSelection => self.llm_fix_selection().await,
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
        let captured = match self.selection.capture_selection().await {
            Ok(Some(sel)) => sel,
            Ok(None) => {
                // Marker survived = copy produced nothing. A last-word fallback
                // uses Backspace, which DELETES an active selection (one
                // Backspace wipes the whole selection) — that is exactly the
                // "selection vanished, nothing inserted" bug. Never fall back
                // to Backspace here.
                return CorrectionOutcome::NoInputFound;
            }
            Err(e) => {
                warn!("selection capture failed: {e}");
                return CorrectionOutcome::Failed(e.to_string());
            }
        };

        self.replace_via_clipboard(captured).await
    }

    /// `LlmFixSelection` hotkey: capture the selection, send the whole text
    /// to the external LLM, replace the selection with the result.
    /// Explicit action only — never part of auto-replace.
    async fn llm_fix_selection(&self) -> CorrectionOutcome {
        let cfg = self.cfg();
        if !cfg.llm.enabled {
            debug!("llm_fix_selection: feature disabled in settings");
            return CorrectionOutcome::Disabled;
        }
        let tc_opt = self.text_corrector.lock().unwrap().clone();
        let Some(tc) = tc_opt else {
            warn!("llm_fix_selection: no text corrector (set llm.base_url)");
            let _ = self
                .notifier
                .notify(
                    "LLM-исправление недоступно: не настроен llm.base_url",
                    NotifyLevel::Warning,
                )
                .await;
            return CorrectionOutcome::Failed("no text corrector".into());
        };

        let captured = match self.selection.capture_selection().await {
            Ok(Some(sel)) => sel,
            Ok(None) => return CorrectionOutcome::NoInputFound,
            Err(e) => {
                warn!("selection capture failed: {e}");
                return CorrectionOutcome::Failed(e.to_string());
            }
        };

        let text = captured.text.trim().to_string();
        if text.is_empty() {
            self.restore_clipboard(captured.saved, &cfg).await;
            return CorrectionOutcome::NoInputFound;
        }

        let _ = self.state_tx.send(CorrectionState::Capturing);
        let corrected = match tc.correct_text(&text).await {
            Ok(t) => t,
            Err(e) => {
                warn!("llm correct_text failed: {e}");
                self.restore_clipboard(captured.saved, &cfg).await;
                let _ = self
                    .notifier
                    .notify("LLM не ответил — текст не изменён", NotifyLevel::Error)
                    .await;
                return CorrectionOutcome::Failed(format!("llm: {e}"));
            }
        };

        let corrected = corrected.trim().to_string();
        if corrected.is_empty() || corrected == text {
            debug!("llm_fix_selection: no changes");
            self.restore_clipboard(captured.saved, &cfg).await;
            let _ = self
                .notifier
                .notify("LLM не нашёл исправлений", NotifyLevel::Info)
                .await;
            return CorrectionOutcome::NoInputFound;
        }

        self.replace_with_text(captured, text, corrected, true).await
    }

    /// Paste `new_text` over the captured selection (no layout convert).
    /// When `arm_undo` is set, one Backspace within [`UNDO_WINDOW`] restores
    /// `original` (the «LLM-fix selection» path).
    async fn replace_with_text(
        &self,
        captured: CapturedSelection,
        original: String,
        new_text: String,
        arm_undo: bool,
    ) -> CorrectionOutcome {
        let cfg = self.cfg();
        let CapturedSelection { saved, .. } = captured;

        let _ = self.state_tx.send(CorrectionState::Replacing);
        if let Err(e) = self.clipboard.put_text(&new_text).await {
            self.restore_clipboard(saved, &cfg).await;
            return CorrectionOutcome::Failed(format!("clipboard put: {e}"));
        }

        let paste_ok = match self.clipboard.get_text().await {
            Ok(t) => t == new_text,
            Err(e) => {
                debug!("clipboard verify: {e}");
                false
            }
        };

        if paste_ok {
            if let Err(e) = self.injector.chord(KeyChord::CtrlV).await {
                self.restore_clipboard(saved, &cfg).await;
                return CorrectionOutcome::Failed(format!("paste: {e}"));
            }
        } else {
            debug!("clipboard offer missing — typing over selection");
            if let Err(e) = self.injector.type_text(&new_text, &LayoutId::new("us")).await {
                self.restore_clipboard(saved, &cfg).await;
                return CorrectionOutcome::Failed(format!("type_text: {e}"));
            }
        }

        tokio::time::sleep(Duration::from_millis(cfg.behavior.paste_settle_ms)).await;
        self.restore_clipboard(saved, &cfg).await;

        if arm_undo {
            self.buffer.set_contents(&new_text);
            *self.undo.lock().unwrap() = Some(UndoState {
                original: original.clone(),
                replacement: new_text.clone(),
                when: Instant::now(),
            });
            debug!("llm undo armed (window {:?})", UNDO_WINDOW);
        }

        CorrectionOutcome::Converted {
            original,
            converted: new_text,
            layout_switched_to: None,
        }
    }

    /// Physical Backspace: roll back a recent LLM selection replace.
    ///
    /// One Backspace = undo the whole replacement (original restored).
    /// Returns `true` when an undo was performed.
    pub async fn on_backspace(&self) -> bool {
        if self.suppress.load(Ordering::SeqCst) {
            return false;
        }
        let undo = {
            let mut g = self.undo.lock().unwrap();
            match g.as_ref() {
                Some(u) if u.when.elapsed() <= UNDO_WINDOW => g.take().unwrap(),
                Some(_) => {
                    *g = None;
                    return false;
                }
                None => return false,
            }
        };
        // Physical Backspace already deleted one character of the replacement.
        let typed = undo.replacement.chars().count();
        let already = 1usize.min(typed);
        let remaining = typed - already;

        self.suppress.store(true, Ordering::SeqCst);
        self.buffer.wait_keys_quiet(50, 500).await;

        if remaining > 0 {
            if let Err(e) = self.injector.backspace(remaining).await {
                warn!("llm undo failed (backspace {remaining}): {e}");
                self.suppress.store(false, Ordering::SeqCst);
                return false;
            }
        }
        let layout = LayoutId::new(match switchfix_domain::detect_lang(&undo.original) {
            Some("ru") => "ru",
            _ => "us",
        });
        if let Err(e) = self.injector.type_text(&undo.original, &layout).await {
            warn!("llm undo failed (type {:?}): {e}", undo.original);
            self.suppress.store(false, Ordering::SeqCst);
            return false;
        }

        self.buffer.set_contents(&undo.original);
        self.suppress.store(false, Ordering::SeqCst);
        debug!(restored = undo.original, "llm undo (backspace after LLM replace)");
        true
    }

    /// The user typed something else — cancel a pending LLM undo.
    pub fn on_typing(&self) {
        if self.suppress.load(Ordering::SeqCst) {
            return;
        }
        let mut g = self.undo.lock().unwrap();
        if g.is_some() {
            debug!("llm undo cancelled (user typed)");
            *g = None;
        }
    }

    /// Clear any pending undo (e.g. on toggle-off).
    pub fn clear_undo(&self) {
        *self.undo.lock().unwrap() = None;
    }

    async fn convert_terminal(&self) -> CorrectionOutcome {
        // Terminal mode: capture selection (or fail), then clear the input
        // line and paste via Ctrl+Shift+V.
        let cfg = self.cfg();
        let (text, saved) = match self.selection.capture_selection().await {
            Ok(Some(sel)) => (sel.text, Some(sel.saved)),
            _ => return CorrectionOutcome::NoInputFound,
        };

        let converted = {
            let tables = self.tables.read().unwrap();
            convert(&text, &tables, None, None)
        };
        let converted = match converted {
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

        let converted = {
            let tables = self.tables.read().unwrap();
            convert(&selected, &tables, None, None)
        };
        let converted = match converted {
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

        // Wayland/X11: if the offer did not stick, Ctrl+V pastes EMPTY and
        // wipes the selection. Verify before pasting; fall back to typing
        // (first keystroke replaces the selection).
        let paste_ok = match self.clipboard.get_text().await {
            Ok(t) => t == converted.converted,
            Err(e) => {
                debug!("clipboard verify: {e}");
                false
            }
        };

        if paste_ok {
            if let Err(e) = self.injector.chord(KeyChord::CtrlV).await {
                self.restore_clipboard(saved, &cfg).await;
                return CorrectionOutcome::Failed(format!("paste: {e}"));
            }
        } else {
            debug!("clipboard offer missing — typing over selection instead of empty paste");
            let target_layout = LayoutId::new(converted.to.as_str().to_string());
            if let Err(e) = self
                .injector
                .type_text(&converted.converted, &target_layout)
                .await
            {
                self.restore_clipboard(saved, &cfg).await;
                return CorrectionOutcome::Failed(format!("type_text: {e}"));
            }
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
        // Last-word hotkey is gone; selection path is the only convert entry.
        let (svc, inj, _clip, buf) = service(None, Some("ghbdtn".into()), false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert_eq!(out, CorrectionOutcome::NoInputFound);
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().all(|l| !l.starts_with("bs:")), "log={log:?}");
        assert!(buf.word.lock().unwrap().is_some(), "buffer unused by selection");
    }

    #[tokio::test]
    async fn last_word_empty_is_no_input() {
        let (svc, _i, _c, _b) = service(None, None, false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert_eq!(out, CorrectionOutcome::NoInputFound);
    }

    #[tokio::test]
    async fn disabled_is_disabled() {
        let (svc, inj, _c, _b) = service(Some("ghbdtn".into()), None, false, false);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
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
    async fn replace_tables_changes_conversion() {
        use std::collections::HashMap;
        use switchfix_domain::{ConversionTable, LayoutId};

        let (svc, _inj, clip, _b) = service(Some("q".into()), None, false, true);
        let mut map = HashMap::new();
        map.insert('q', 'Z');
        let forward = ConversionTable::new(LayoutId::new("us"), LayoutId::new("ru"), map);
        svc.replace_tables(ConversionSet::new(vec![forward.clone(), forward.reverse()]));

        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert!(out.is_success(), "{out:?}");
        let clog = clip.log.lock().unwrap().clone();
        assert!(clog.iter().any(|l| l == "put:Z"), "replaced map: {clog:?}");
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
    async fn selection_no_capture_does_not_eat_selection() {
        let (svc, inj, _c, _b) = service(None, Some("руддщ".into()), false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert_eq!(out, CorrectionOutcome::NoInputFound);
        let log = inj.log.lock().unwrap().clone();
        assert!(
            !log.iter().any(|l| l.starts_with("bs:")),
            "must not backspace over a possible selection: {log:?}"
        );
    }

    #[tokio::test]
    async fn layout_fail_still_reports_conversion() {
        let (svc, _i, _c, _b) = service(Some("ghbdtn".into()), None, true, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
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
        let (svc, inj, _c, _b) = service(Some("ghbdtn".into()), None, false, true);
        let out = svc.handle(CorrectionRequest::ConvertTerminal).await;
        assert!(out.is_success(), "{out:?}");
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l.contains("CtrlA")), "log={log:?}");
        assert!(log.iter().any(|l| l.contains("CtrlK")), "log={log:?}");
        assert!(log.iter().any(|l| l.contains("CtrlShiftV")), "log={log:?}");
    }

    struct FakeTextCorrector {
        fixed: String,
    }

    #[async_trait]
    impl TextCorrector for FakeTextCorrector {
        fn name(&self) -> &'static str {
            "fake-openai"
        }
        async fn correct_text(&self, _text: &str) -> Result<String, TextFixError> {
            Ok(self.fixed.clone())
        }
    }

    #[tokio::test]
    async fn llm_fix_selection_replaces_with_corrected_text() {
        let (svc, inj, clip, _b) = service(Some("он ачала говорить".into()), None, false, true);
        svc.set_text_corrector(Arc::new(FakeTextCorrector {
            fixed: "он начал говорить".into(),
        }));
        let out = svc.handle(CorrectionRequest::LlmFixSelection).await;
        assert!(out.is_success(), "{out:?}");
        assert_eq!(
            out,
            CorrectionOutcome::Converted {
                original: "он ачала говорить".into(),
                converted: "он начал говорить".into(),
                layout_switched_to: None,
            }
        );
        // Corrected text went to the clipboard (then restored afterwards).
        let clog = clip.log.lock().unwrap().clone();
        assert!(
            clog.iter().any(|l| l == "put:он начал говорить"),
            "clipboard log={clog:?}"
        );
        let log = inj.log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l.contains("CtrlV")), "log={log:?}");
    }

    #[tokio::test]
    async fn llm_fix_selection_no_changes_is_noop() {
        let (svc, _i, _c, _b) = service(Some("уже хорошо".into()), None, false, true);
        svc.set_text_corrector(Arc::new(FakeTextCorrector {
            fixed: "уже хорошо".into(),
        }));
        let out = svc.handle(CorrectionRequest::LlmFixSelection).await;
        assert_eq!(out, CorrectionOutcome::NoInputFound);
    }

    #[tokio::test]
    async fn llm_fix_selection_respects_disable_flag() {
        let injector = Arc::new(FakeInjector::default());
        let clipboard = Arc::new(FakeClipboard::default());
        let selection = Arc::new(FakeSelection {
            text: Some("текст".into()),
        });
        let buffer = Arc::new(FakeBuffer::default());
        let layouts = Arc::new(FakeLayouts { fail: false });
        let notifier = Arc::new(FakeNotifier::default());
        let (_tx, rx) = watch::channel({
            let mut c = AppConfig::default();
            c.llm.enabled = false;
            c
        });
        let svc = CorrectionService::new(
            injector,
            clipboard,
            selection,
            buffer,
            layouts,
            notifier,
            ConversionSet::en_ru(),
            rx,
            true,
        );
        svc.set_text_corrector(Arc::new(FakeTextCorrector {
            fixed: "другое".into(),
        }));
        let out = svc.handle(CorrectionRequest::LlmFixSelection).await;
        assert_eq!(out, CorrectionOutcome::Disabled);
    }

    #[tokio::test]
    async fn llm_undo_restores_original_on_backspace() {
        let (svc, inj, _clip, _b) = service(Some("он ачала".into()), None, false, true);
        svc.set_text_corrector(Arc::new(FakeTextCorrector {
            fixed: "он начал".into(),
        }));
        let out = svc.handle(CorrectionRequest::LlmFixSelection).await;
        assert!(out.is_success(), "{out:?}");
        inj.log.lock().unwrap().clear();

        assert!(svc.on_backspace().await, "backspace must undo");
        let log = inj.log.lock().unwrap().clone();
        // replacement "он начал" = 8 chars; physical BS ate 1 → backspace(7).
        assert!(log.iter().any(|l| l == "bs:7"), "log={log:?}");
        assert!(
            log.iter().any(|l| l.starts_with("type:он ачала")),
            "log={log:?}"
        );
        // Second backspace is a plain delete (undo already consumed).
        inj.log.lock().unwrap().clear();
        assert!(!svc.on_backspace().await);
        assert!(inj.log.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn llm_undo_expired_window_is_noop() {
        let (svc, inj, _clip, _b) = service(Some("он ачала".into()), None, false, true);
        svc.set_text_corrector(Arc::new(FakeTextCorrector {
            fixed: "он начал".into(),
        }));
        let _ = svc.handle(CorrectionRequest::LlmFixSelection).await;
        // Expire the window artificially.
        {
            let mut g = svc.undo.lock().unwrap();
            if let Some(u) = g.as_mut() {
                u.when = Instant::now() - Duration::from_secs(10);
            }
        }
        inj.log.lock().unwrap().clear();
        assert!(!svc.on_backspace().await);
        assert!(inj.log.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn llm_undo_cleared_by_typing() {
        let (svc, inj, _clip, _b) = service(Some("он ачала".into()), None, false, true);
        svc.set_text_corrector(Arc::new(FakeTextCorrector {
            fixed: "он начал".into(),
        }));
        let _ = svc.handle(CorrectionRequest::LlmFixSelection).await;
        svc.on_typing();
        inj.log.lock().unwrap().clear();
        assert!(!svc.on_backspace().await);
        assert!(inj.log.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn llm_undo_not_armed_for_convert() {
        let (svc, inj, _clip, _b) = service(Some("ghbdtn".into()), None, false, true);
        let out = svc.handle(CorrectionRequest::ConvertSelection).await;
        assert!(out.is_success(), "{out:?}");
        inj.log.lock().unwrap().clear();
        assert!(!svc.on_backspace().await);
        assert!(inj.log.lock().unwrap().is_empty());
    }
}
