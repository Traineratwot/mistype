//! Spellcheck + word-completion orchestration.
//!
//! Reacts to typing-buffer word events and the `FixSpelling` hotkey:
//! builds candidates (spell + layout-convert), auto-fixes or shows the bar,
//! and replaces words via `TextInjector` (backspace + type_text).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use switchfix_domain::{
    convert, detect_lang, is_auto_fixable, one_step_neighbors, preserve_case, rank, should_check,
    ConversionSet, Suggestion, SuggestionKind,
};
use tokio::sync::watch;
use tracing::debug;

use crate::config::AppConfig;
use crate::ports::{
    FocusMonitor, SpellChecker, SuggestionPresenter, SuggestionUi, TextInjector, TypingBufferPort,
    WordEvent,
};

/// What happened — the caller uses this to play a sound / flash the tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellAction {
    /// Nothing to do (correct word, disabled, excluded app, …).
    None,
    /// Hints are now visible in the suggestion bar.
    ShowedHints,
    /// A word (or fragment) was replaced.
    Replaced { from: String, to: String },
    /// Word was added to the personal dictionary.
    Learned { word: String },
    /// Bar was hidden without a change.
    Dismissed,
}

/// Pending suggestions currently shown (or ready for the hotkey path).
#[derive(Debug, Clone)]
struct Pending {
    /// Word/fragment being replaced (original spelling).
    original: String,
    /// How many chars to erase (word + optional trailing separator).
    erase_len: usize,
    /// Separator to re-type after the replacement.
    trailing: String,
    slots: Vec<Suggestion>,
}

/// Orchestrates spellcheck, completion and word replacement.
pub struct SpellService {
    spell: Arc<dyn SpellChecker>,
    injector: Arc<dyn TextInjector>,
    presenter: Arc<dyn SuggestionPresenter>,
    buffer: Arc<dyn TypingBufferPort>,
    tables: ConversionSet,
    config: watch::Receiver<AppConfig>,
    focus: Option<Arc<dyn FocusMonitor>>,
    enabled: AtomicBool,
    pending: Mutex<Option<Pending>>,
    /// Last completed word — used by the `FixSpelling` hotkey.
    last_word: Mutex<Option<(String, usize, char)>>,
}

impl SpellService {
    pub fn new(
        spell: Arc<dyn SpellChecker>,
        injector: Arc<dyn TextInjector>,
        presenter: Arc<dyn SuggestionPresenter>,
        buffer: Arc<dyn TypingBufferPort>,
        tables: ConversionSet,
        config: watch::Receiver<AppConfig>,
        enabled: bool,
    ) -> Self {
        Self::with_focus(spell, injector, presenter, buffer, tables, config, None, enabled)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_focus(
        spell: Arc<dyn SpellChecker>,
        injector: Arc<dyn TextInjector>,
        presenter: Arc<dyn SuggestionPresenter>,
        buffer: Arc<dyn TypingBufferPort>,
        tables: ConversionSet,
        config: watch::Receiver<AppConfig>,
        focus: Option<Arc<dyn FocusMonitor>>,
        enabled: bool,
    ) -> Self {
        Self {
            spell,
            injector,
            presenter,
            buffer,
            tables,
            config,
            focus,
            enabled: AtomicBool::new(enabled),
            pending: Mutex::new(None),
            last_word: Mutex::new(None),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        if !on {
            *self.pending.lock().unwrap() = None;
            *self.last_word.lock().unwrap() = None;
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Feed a typing-buffer event.
    pub async fn on_word_event(&self, ev: WordEvent) -> SpellAction {
        if !self.is_enabled() {
            return SpellAction::None;
        }
        let cfg = self.config.borrow().clone();
        if !cfg.spell.enabled || cfg.spell.mode == "off" {
            return SpellAction::None;
        }
        if self.should_skip_focus(&cfg).await {
            return SpellAction::None;
        }

        match ev {
            WordEvent::Completed { word, separator } => {
                self.on_word_completed(word.text, word.char_len, separator, &cfg)
                    .await
            }
            WordEvent::Updated(frag) => self.on_fragment_updated(frag, &cfg).await,
        }
    }

    /// `Ctrl+F2` — replace the last word with the top candidate, no UI.
    pub async fn fix_last_word(&self) -> SpellAction {
        if !self.is_enabled() {
            return SpellAction::None;
        }
        let cfg = self.config.borrow().clone();
        if !cfg.spell.enabled {
            return SpellAction::None;
        }

        // Prefer the stored completed word; fall back to the buffer fragment.
        let (word, erase_len, trailing) = {
            let last = self.last_word.lock().unwrap().clone();
            if let Some((w, len, sep)) = last {
                (w, len + 1, sep.to_string())
            } else {
                // Fragment still being typed: erase just the fragment, no separator.
                match self.buffer.peek_current_word() {
                    Some(w) if !w.is_empty() => {
                        let len = w.chars().count();
                        (w, len, String::new())
                    }
                    _ => return SpellAction::None,
                }
            }
        };

        let Some(mut candidates) = self.candidates_for(&word, &cfg, false) else {
            return SpellAction::None;
        };
        candidates = rank(&word, candidates);
        let Some(top) = candidates.first().cloned() else {
            return SpellAction::None;
        };
        let replacement = preserve_case(&word, &top.text);
        self.apply_replace(&word, &replacement, erase_len, &trailing)
            .await
    }

    /// Accept slot `n` (1-based) from the visible bar.
    pub async fn accept_slot(&self, n: u8) -> SpellAction {
        let pending = self.pending.lock().unwrap().clone();
        let Some(p) = pending else {
            return SpellAction::None;
        };
        let idx = n.saturating_sub(1) as usize;
        let Some(slot) = p.slots.get(idx).cloned() else {
            return SpellAction::None;
        };
        let replacement = preserve_case(&p.original, &slot.text);
        let _ = self.presenter.hide().await;
        *self.pending.lock().unwrap() = None;
        self.apply_replace(&p.original, &replacement, p.erase_len, &p.trailing)
            .await
    }

    /// `Ctrl+0` — learn the original word.
    pub async fn add_to_dictionary(&self) -> SpellAction {
        let pending = self.pending.lock().unwrap().clone();
        let Some(p) = pending else {
            return SpellAction::None;
        };
        match self.spell.add_to_dictionary(&p.original) {
            Ok(()) => {
                let _ = self.presenter.hide().await;
                *self.pending.lock().unwrap() = None;
                SpellAction::Learned { word: p.original }
            }
            Err(e) => {
                debug!("add_to_dictionary failed: {e}");
                SpellAction::None
            }
        }
    }

    /// Hide the bar.
    pub async fn dismiss(&self) -> SpellAction {
        let had = self.pending.lock().unwrap().take().is_some();
        let _ = self.presenter.hide().await;
        if had {
            SpellAction::Dismissed
        } else {
            SpellAction::None
        }
    }

    // ----- internals -----

    async fn should_skip_focus(&self, cfg: &AppConfig) -> bool {
        let Some(focus) = &self.focus else {
            return false;
        };
        let Some(win) = focus.active_window().await else {
            return false;
        };
        let app = win.app_id.to_lowercase();
        if cfg.behavior.excluded_apps.iter().any(|e| e.to_lowercase() == app) {
            return true;
        }
        if win.fullscreen && cfg.behavior.disable_in_fullscreen {
            return true;
        }
        false
    }

    async fn on_word_completed(
        &self,
        word: String,
        char_len: usize,
        separator: char,
        cfg: &AppConfig,
    ) -> SpellAction {
        *self.last_word.lock().unwrap() = Some((word.clone(), char_len, separator));

        if !should_check(&word, cfg.spell.min_word_len) {
            let _ = self.dismiss_if_showing().await;
            return SpellAction::None;
        }

        let Some(candidates) = self.candidates_for(&word, cfg, false) else {
            let _ = self.dismiss_if_showing().await;
            return SpellAction::None;
        };
        if candidates.is_empty() {
            let _ = self.dismiss_if_showing().await;
            return SpellAction::None;
        }

        let ranked = rank(&word, candidates);
        let mode = cfg.spell.mode.as_str();

        if mode == "auto" && is_auto_fixable(&ranked, &word, cfg.spell.auto_fix_min_len) {
            let top = &ranked[0];
            let replacement = preserve_case(&word, &top.text);
            *self.pending.lock().unwrap() = None;
            let _ = self.presenter.hide().await;
            return self
                .apply_replace(&word, &replacement, char_len + 1, &separator.to_string())
                .await;
        }

        if mode == "hotkey" {
            // No bar; the hotkey uses last_word.
            return SpellAction::None;
        }

        // hint (and auto with multiple candidates) → show the bar.
        self.show_bar(word, char_len + 1, &separator.to_string(), ranked, false, cfg)
            .await
    }

    async fn on_fragment_updated(&self, frag: String, cfg: &AppConfig) -> SpellAction {
        if cfg.spell.mode != "hint" && cfg.spell.mode != "auto" {
            return SpellAction::None;
        }
        if frag.chars().count() < cfg.spell.completion_min_chars {
            return SpellAction::None;
        }

        let Some(candidates) = self.candidates_for(&frag, cfg, true) else {
            return SpellAction::None;
        };
        if candidates.is_empty() {
            return SpellAction::None;
        }
        let ranked = rank(&frag, candidates);
        // Completions replace only the fragment (still being typed).
        self.show_bar(frag, 0, "", ranked, true, cfg).await
    }

    /// Build ranked-able candidates for `word`.
    ///
    /// `completions_only` switches to prefix completion instead of spell-fix.
    fn candidates_for(
        &self,
        word: &str,
        cfg: &AppConfig,
        completions_only: bool,
    ) -> Option<Vec<Suggestion>> {
        let lang = detect_lang(word)?;
        if !cfg.spell.languages.iter().any(|l| l == lang) {
            return None;
        }
        let n = cfg.spell.suggest_count.saturating_add(3); // over-fetch, rank later

        let mut out: Vec<Suggestion> = Vec::new();

        if completions_only {
            for c in self.spell.complete(word, lang, n) {
                out.push(Suggestion::new(c, SuggestionKind::Completion, word));
            }
            return Some(out);
        }

        // Correct word → no spell-fix candidates (layout-convert still worth a try
        // only when the word is *incorrect* in its own script).
        if self.spell.is_correct(word, lang) {
            return None;
        }

        // Layout-convert candidate: `ghbdtn` → `привет` when that is a real word.
        if let Ok(conv) = convert(word, &self.tables, None, None) {
            let converted = conv.converted;
            if let Some(c_lang) = detect_lang(&converted) {
                if self.spell.is_correct(&converted, c_lang) {
                    out.push(Suggestion::new(
                        converted,
                        SuggestionKind::LayoutConvert,
                        word,
                    ));
                }
            }
        }

        for c in self.spell.suggest(word, lang, n) {
            out.push(Suggestion::new(c, SuggestionKind::SpellFix, word));
        }

        // zspell ranking can miss the obvious edit; add one-step neighbors
        // that the dictionary accepts (drop-last, drop-first, transpose-tail).
        for cand in one_step_neighbors(word) {
            if out.iter().any(|s| s.text == cand) {
                continue;
            }
            if let Some(m_lang) = detect_lang(&cand) {
                if self.spell.is_correct(&cand, m_lang) {
                    out.push(Suggestion::new(cand, SuggestionKind::SpellFix, word));
                }
            }
        }
        Some(out)
    }

    async fn show_bar(
        &self,
        original: String,
        erase_len: usize,
        trailing: &str,
        slots: Vec<Suggestion>,
        is_completion: bool,
        cfg: &AppConfig,
    ) -> SpellAction {
        let n = cfg.spell.suggest_count;
        let slots: Vec<Suggestion> = slots.into_iter().take(n).collect();
        if slots.is_empty() {
            return SpellAction::None;
        }

        let ui = SuggestionUi {
            original: original.clone(),
            slots: slots.clone(),
            can_add_to_dict: !is_completion && cfg.spell.mode != "hotkey",
            timeout_ms: cfg.spell.bar_timeout_ms,
        };
        let _ = self.presenter.show(ui).await;
        *self.pending.lock().unwrap() = Some(Pending {
            original,
            erase_len,
            trailing: trailing.to_string(),
            slots,
        });
        SpellAction::ShowedHints
    }

    async fn dismiss_if_showing(&self) -> SpellAction {
        self.dismiss().await
    }

    async fn apply_replace(
        &self,
        original: &str,
        replacement: &str,
        erase_len: usize,
        trailing: &str,
    ) -> SpellAction {
        if replacement == original {
            return SpellAction::None;
        }
        // Layout of the replacement: type as the *target* script's layout.
        let layout = switchfix_domain::LayoutId::new(match detect_lang(replacement) {
            Some("ru") => "ru",
            _ => "us",
        });

        if erase_len > 0 {
            if let Err(e) = self.injector.backspace(erase_len).await {
                debug!("backspace failed: {e}");
                return SpellAction::None;
            }
        }
        if let Err(e) = self.injector.type_text(replacement, &layout).await {
            debug!("type_text failed: {e}");
            return SpellAction::None;
        }
        if !trailing.is_empty() {
            let sep_layout = switchfix_domain::LayoutId::new("us");
            let _ = self.injector.type_text(trailing, &sep_layout).await;
        }

        debug!(from = original, to = replacement, "spell replaced");
        SpellAction::Replaced {
            from: original.to_string(),
            to: replacement.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{
        InjectError, LayoutError, NotifyError, SavedClipboard, SpellError,
    };
    use std::sync::Mutex as StdMutex;
    use switchfix_domain::{KeyChord, LayoutId, LayoutIndex, LayoutInfo, WordSpan};
    use tokio::sync::broadcast;

    #[derive(Default)]
    struct FakeInjector {
        log: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl TextInjector for FakeInjector {
        async fn backspace(&self, count: usize) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("bs:{count}"));
            Ok(())
        }
        async fn type_text(&self, text: &str, layout: &LayoutId) -> Result<(), InjectError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("type:{text}:{}", layout.as_str()));
            Ok(())
        }
        async fn chord(&self, _combo: KeyChord) -> Result<(), InjectError> {
            Ok(())
        }
    }

    struct FakeSpell {
        correct: Vec<String>,
        suggestions: Vec<(String, Vec<String>)>,
        completions: Vec<(String, Vec<String>)>,
        learned: StdMutex<Vec<String>>,
    }

    impl FakeSpell {
        fn new() -> Self {
            Self {
                correct: vec!["привет".into(), "hello".into()],
                suggestions: vec![
                    ("приветт".into(), vec!["привет".into()]),
                    ("ghbdtn".into(), vec!["ghbdtnn".into()]),
                ],
                completions: vec![("при".into(), vec!["привет".into(), "прибор".into()])],
                learned: StdMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl SpellChecker for FakeSpell {
        fn is_correct(&self, word: &str, _lang: &str) -> bool {
            self.correct.iter().any(|c| c == word)
        }
        fn suggest(&self, word: &str, _lang: &str, n: usize) -> Vec<String> {
            self.suggestions
                .iter()
                .find(|(w, _)| w == word)
                .map(|(_, s)| s.iter().take(n).cloned().collect())
                .unwrap_or_default()
        }
        fn complete(&self, prefix: &str, _lang: &str, n: usize) -> Vec<String> {
            self.completions
                .iter()
                .find(|(p, _)| p == prefix)
                .map(|(_, s)| s.iter().take(n).cloned().collect())
                .unwrap_or_default()
        }
        fn add_to_dictionary(&self, word: &str) -> Result<(), SpellError> {
            self.learned.lock().unwrap().push(word.to_string());
            Ok(())
        }
        fn languages(&self) -> Vec<String> {
            vec!["ru".into(), "en".into()]
        }
    }

    #[derive(Default)]
    struct FakePresenter {
        visible: AtomicBool,
        shown: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl SuggestionPresenter for FakePresenter {
        async fn show(&self, model: SuggestionUi) -> Result<(), NotifyError> {
            self.shown.lock().unwrap().push(model.original.clone());
            self.visible.store(true, Ordering::SeqCst);
            Ok(())
        }
        async fn hide(&self) -> Result<(), NotifyError> {
            self.visible.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn is_visible(&self) -> bool {
            self.visible.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct FakeBuffer {
        word: StdMutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl TypingBufferPort for FakeBuffer {
        fn take_last_word(&self) -> Option<WordSpan> {
            let t = self.word.lock().unwrap().take()?;
            let char_len = t.chars().count();
            Some(WordSpan { text: t, char_len })
        }
        fn clear(&self) {
            *self.word.lock().unwrap() = None;
        }
        fn is_enabled(&self) -> bool {
            true
        }
        fn push_char(&self, _c: char) {}
        fn backspace(&self) {}
        fn peek_current_word(&self) -> Option<String> {
            self.word.lock().unwrap().clone()
        }
        fn word_events(&self) -> broadcast::Receiver<WordEvent> {
            let (tx, rx) = broadcast::channel(1);
            drop(tx);
            rx
        }
    }

    fn setup(mode: &str) -> (SpellService, Arc<FakeInjector>, Arc<FakePresenter>) {
        let injector = Arc::new(FakeInjector::default());
        let presenter = Arc::new(FakePresenter::default());
        let spell = Arc::new(FakeSpell::new());
        let buffer = Arc::new(FakeBuffer::default());
        let (_tx, rx) = watch::channel({
            let mut c = AppConfig::default();
            c.spell.mode = mode.to_string();
            c
        });
        let svc = SpellService::new(
            spell,
            injector.clone(),
            presenter.clone(),
            buffer,
            ConversionSet::en_ru(),
            rx,
            true,
        );
        (svc, injector, presenter)
    }

    #[tokio::test]
    async fn completed_typo_shows_hints() {
        let (svc, _i, p) = setup("hint");
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "приветт".into(),
                    char_len: 7,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(action, SpellAction::ShowedHints);
        assert!(p.is_visible());
    }

    #[test]
    fn one_step_neighbors_covers_common_typos() {
        let n = one_step_neighbors("приветт");
        assert!(n.contains(&"привет".to_string()), "drop last: {n:?}");
        let n = one_step_neighbors("teh");
        assert!(n.contains(&"the".to_string()) || n.contains(&"eh".to_string()), "got {n:?}");
    }

    #[tokio::test]
    async fn accept_slot_replaces_word() {
        let (svc, i, p) = setup("hint");
        svc.on_word_event(WordEvent::Completed {
            word: WordSpan {
                text: "приветт".into(),
                char_len: 7,
            },
            separator: ' ',
        })
        .await;
        let action = svc.accept_slot(1).await;
        assert_eq!(
            action,
            SpellAction::Replaced {
                from: "приветт".into(),
                to: "привет".into(),
            }
        );
        assert!(!p.is_visible());
        let log = i.log.lock().unwrap().clone();
        assert_eq!(log, vec!["bs:8", "type:привет:ru", "type: :us"]);
    }

    #[tokio::test]
    async fn layout_convert_is_first_candidate() {
        let (svc, i, _p) = setup("hint");
        // ghbdtn is misspelled in EN; convert → привет which is correct in RU.
        svc.on_word_event(WordEvent::Completed {
            word: WordSpan {
                text: "ghbdtn".into(),
                char_len: 6,
            },
            separator: ' ',
        })
        .await;
        let action = svc.accept_slot(1).await;
        assert_eq!(
            action,
            SpellAction::Replaced {
                from: "ghbdtn".into(),
                to: "привет".into(),
            }
        );
        let log = i.log.lock().unwrap().clone();
        assert_eq!(log[1], "type:привет:ru");
    }

    #[tokio::test]
    async fn auto_mode_replaces_single_close_candidate() {
        let (svc, i, p) = setup("auto");
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "приветт".into(),
                    char_len: 7,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(
            action,
            SpellAction::Replaced {
                from: "приветт".into(),
                to: "привет".into(),
            }
        );
        assert!(!p.is_visible());
        let log = i.log.lock().unwrap().clone();
        assert_eq!(log[0], "bs:8");
    }

    #[tokio::test]
    async fn hotkey_mode_shows_no_bar() {
        let (svc, _i, p) = setup("hotkey");
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "приветт".into(),
                    char_len: 7,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(action, SpellAction::None);
        assert!(!p.is_visible());

        let action = svc.fix_last_word().await;
        assert!(matches!(action, SpellAction::Replaced { .. }));
    }

    #[tokio::test]
    async fn correct_word_is_ignored() {
        let (svc, _i, p) = setup("hint");
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "привет".into(),
                    char_len: 6,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(action, SpellAction::None);
        assert!(!p.is_visible());
    }

    #[tokio::test]
    async fn completion_offers_prefix_matches() {
        let (svc, _i, p) = setup("hint");
        let action = svc.on_word_event(WordEvent::Updated("при".into())).await;
        assert_eq!(action, SpellAction::ShowedHints);
        assert!(p.is_visible());
    }

    #[tokio::test]
    async fn add_to_dictionary_learns() {
        let (svc, _i, _p) = setup("hint");
        svc.on_word_event(WordEvent::Completed {
            word: WordSpan {
                text: "приветт".into(),
                char_len: 7,
            },
            separator: ' ',
        })
        .await;
        let action = svc.add_to_dictionary().await;
        assert_eq!(action, SpellAction::Learned { word: "приветт".into() });
    }

    #[tokio::test]
    async fn disabled_spell_does_nothing() {
        let (svc, _i, p) = setup("hint");
        svc.set_enabled(false);
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "приветт".into(),
                    char_len: 7,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(action, SpellAction::None);
        assert!(!p.is_visible());
    }

    // Silence unused import warnings for re-exports used by fakes.
    #[allow(dead_code)]
    fn _unused(_: LayoutError, _: LayoutIndex, _: LayoutInfo, _: SavedClipboard) {}
}
