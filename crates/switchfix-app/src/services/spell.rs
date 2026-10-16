//! Spellcheck + word-completion orchestration.
//!
//! Reacts to typing-buffer word events and the `FixSpelling` hotkey:
//! builds candidates (spell + layout-convert), auto-fixes or shows the bar,
//! and replaces words via `TextInjector` (backspace + type_text).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use switchfix_domain::{
    confusable_neighbors, convert, detect_lang, edit1_neighbors, filter_candidates,
    one_step_neighbors, preserve_case, rank, should_auto_apply, should_check, ConversionSet,
    Suggestion, SuggestionKind,
};
use tokio::sync::watch;
use tracing::debug;

use crate::config::AppConfig;
use crate::ports::{
    ContextCorrector, FocusMonitor, SpellChecker, SuggestionPresenter, SuggestionUi, TextInjector,
    TypingBufferPort, WordEvent,
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
    /// Optional hybrid layer (LanguageTool / LLM). Never blocks the dict path.
    context: Mutex<Option<Arc<dyn ContextCorrector>>>,
    injector: Arc<dyn TextInjector>,
    presenter: Arc<dyn SuggestionPresenter>,
    buffer: Arc<dyn TypingBufferPort>,
    tables: std::sync::RwLock<ConversionSet>,
    config: watch::Receiver<AppConfig>,
    focus: Option<Arc<dyn FocusMonitor>>,
    enabled: AtomicBool,
    pending: Mutex<Option<Pending>>,
    /// Last completed word — used by the `FixSpelling` hotkey.
    last_word: Mutex<Option<(String, usize, char)>>,
    /// Ignore word events while we inject a replacement (they are our own keys).
    suppress: AtomicBool,
    /// After a replace, block auto-replace until the user types a new word.
    /// Prevents "Space → replace → Space → replace again" on a stale buffer.
    auto_block: AtomicBool,
    /// When the last replace happened (for logs / future debounce).
    last_replace: Mutex<Option<Instant>>,
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
            context: Mutex::new(None),
            injector,
            presenter,
            buffer,
            tables: std::sync::RwLock::new(tables),
            config,
            focus,
            enabled: AtomicBool::new(enabled),
            pending: Mutex::new(None),
            last_word: Mutex::new(None),
            suppress: AtomicBool::new(false),
            auto_block: AtomicBool::new(false),
            last_replace: Mutex::new(None),
        }
    }

    /// Install a hybrid context backend (LanguageTool / LLM). Optional.
    pub fn set_context_corrector(&self, ctx: Arc<dyn ContextCorrector>) {
        debug!("spell context backend: {}", ctx.name());
        *self.context.lock().unwrap() = Some(ctx);
    }

    /// Swap conversion tables at runtime (config reload).
    pub fn replace_tables(&self, tables: ConversionSet) {
        *self.tables.write().unwrap() = tables;
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
        if self.suppress.load(Ordering::SeqCst) {
            debug!("spell skip: suppressed (injected keys)");
            return SpellAction::None;
        }
        if !self.is_enabled() {
            debug!("spell skip: service disabled (toggle)");
            return SpellAction::None;
        }
        let cfg = self.config.borrow().clone();
        if !cfg.spell.enabled || cfg.spell.mode == "off" {
            debug!(
                "spell skip: config enabled={} mode={}",
                cfg.spell.enabled, cfg.spell.mode
            );
            return SpellAction::None;
        }
        if self.should_skip_focus(&cfg).await {
            debug!("spell skip: excluded app / fullscreen");
            return SpellAction::None;
        }

        match ev {
            WordEvent::Completed { word, separator } => {
                debug!("word completed: {:?} sep={:?}", word.text, separator);
                self.on_word_completed(word.text, word.char_len, separator, &cfg)
                    .await
            }
            WordEvent::Updated(frag) => {
                // Real typing (not our inject) re-arms auto-replace.
                if !frag.is_empty() {
                    self.auto_block.store(false, Ordering::SeqCst);
                }
                debug!("word fragment: {:?}", frag);
                self.on_fragment_updated(frag, &cfg).await
            }
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
        // `erase_len` is the word only — trailing separator is stepped over,
        // never deleted (and the space before the word is never touched).
        let (word, erase_len, trailing) = {
            let last = self.last_word.lock().unwrap().clone();
            if let Some((w, len, sep)) = last {
                (w, len, sep.to_string())
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

        let Some(mut candidates) = self.candidates_for(&word, &cfg, false).await else {
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
            debug!("spell skip word {:?}: should_check=false (min_len={})", word, cfg.spell.min_word_len);
            let _ = self.dismiss_if_showing().await;
            return SpellAction::None;
        }

        let Some(candidates) = self.candidates_for(&word, cfg, false).await else {
            debug!("spell skip word {:?}: candidates_for=None (lang/config/correct)", word);
            let _ = self.dismiss_if_showing().await;
            return SpellAction::None;
        };
        if candidates.is_empty() {
            debug!("spell skip word {:?}: no candidates", word);
            let _ = self.dismiss_if_showing().await;
            return SpellAction::None;
        }

        let ranked = rank(&word, candidates);
        let mode = cfg.spell.mode.as_str();
        debug!(
            "spell word {:?} mode={} ranked={:?}",
            word,
            mode,
            ranked.iter().map(|s| s.text.as_str()).collect::<Vec<_>>()
        );

        // Auto mode: apply the top candidate as soon as the word is finished
        // (space / punctuation) when the fix is high-confidence.
        // Never two auto-replaces in a row — wait for the user to type again.
        if mode == "auto" {
            if self.auto_block.load(Ordering::SeqCst) {
                debug!("spell auto: blocked (previous replace, wait for typing)");
                return SpellAction::None;
            }
            if should_auto_apply(&ranked, &word, cfg.spell.auto_fix_min_len) {
                let top = &ranked[0];
                let replacement = preserve_case(&word, &top.text);
                debug!("spell auto-fix {:?} -> {:?}", word, replacement);
                *self.pending.lock().unwrap() = None;
                let _ = self.presenter.hide().await;
                // Word only; the Space the user pressed stays after the word.
                return self
                    .apply_replace(&word, &replacement, char_len, &separator.to_string())
                    .await;
            }
            debug!(
                "spell auto: not auto-applying (candidates={} min_len={}) — showing hints",
                ranked.len(),
                cfg.spell.auto_fix_min_len
            );
        }

        if mode == "hotkey" {
            // No bar; the hotkey uses last_word.
            return SpellAction::None;
        }

        // hint (and auto with multiple candidates) → show the bar. Never replace.
        // Word-only erase: the trailing separator stays where the user typed it.
        self.show_bar(word, char_len, &separator.to_string(), ranked, false, cfg)
            .await
    }

    async fn on_fragment_updated(&self, frag: String, cfg: &AppConfig) -> SpellAction {
        if cfg.spell.mode != "hint" && cfg.spell.mode != "auto" {
            return SpellAction::None;
        }
        if frag.chars().count() < cfg.spell.completion_min_chars {
            return SpellAction::None;
        }

        let Some(candidates) = self.candidates_for(&frag, cfg, true).await else {
            return SpellAction::None;
        };
        if candidates.is_empty() {
            return SpellAction::None;
        }
        let ranked = rank(&frag, candidates);
        // Completions: erase only the typed fragment (no trailing separator yet).
        let frag_len = frag.chars().count();
        self.show_bar(frag, frag_len, "", ranked, true, cfg).await
    }

    /// Build ranked-able candidates for `word`.
    ///
    /// Hybrid layers:
    /// 1. layout-convert (core SwitchFix)
    /// 2. dictionary suggest (compat zspell)
    /// 3. expanded edit-1 + confusable neighbors filtered by the dictionary
    /// 4. optional ContextCorrector (LT / LLM) — can also veto false positives
    ///
    /// `completions_only` switches to prefix completion instead of spell-fix.
    async fn candidates_for(
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
        let use_hybrid = cfg.spell.backend != "dict";

        let mut out: Vec<Suggestion> = Vec::new();

        if completions_only {
            for c in self.spell.complete(word, lang, n) {
                out.push(Suggestion::new(c, SuggestionKind::Completion, word));
            }
            return Some(out);
        }

        let dict_ok = self.spell.is_correct(word, lang);

        // One context query per word (LT/LLM is the expensive layer).
        let ctx_fixes = if use_hybrid && !completions_only {
            self.context_fixes(word, lang, cfg).await
        } else {
            Vec::new()
        };

        // Context veto: dictionary said "wrong", but the engine says it's fine
        // (classic Russian morphology false-positive). Don't show a bar at all.
        if !dict_ok && ctx_fixes.iter().any(|f| f.is_ok) {
            debug!(
                "spell skip {:?}: context says word is ok (dict false positive)",
                word
            );
            return None;
        }

        // High-confidence context fix (with layout-convert) — skip the dict pile.
        if !dict_ok {
            let strong: Vec<Suggestion> = ctx_fixes
                .iter()
                .filter(|f| !f.is_ok && f.confidence >= 0.75)
                .map(|f| Suggestion::new(f.text.clone(), SuggestionKind::SpellFix, word))
                .collect();
            if !strong.is_empty() {
                let conv = self.layout_candidate(word);
                let mut merged = Vec::new();
                if let Some(c) = conv {
                    merged.push(c);
                }
                merged.extend(strong);
                return Some(filter_candidates(word, merged, 3));
            }
        }

        // Correct word → no spell-fix candidates (layout-convert still worth a try
        // only when the word is *incorrect* in its own script).
        if dict_ok {
            debug!("word {:?} is correct in '{}', no spell-fix", word, lang);
            return None;
        }

        // Layout-convert candidate: `ghbdtn` → `привет` when that is a real word.
        if let Some(c) = self.layout_candidate(word) {
            out.push(c);
        }

        for c in self.spell.suggest(word, lang, n) {
            out.push(Suggestion::new(c, SuggestionKind::SpellFix, word));
        }

        if use_hybrid {
            // Broad neighborhood: the cheap one-step set misses most typos.
            for cand in edit1_neighbors(word)
                .into_iter()
                .chain(confusable_neighbors(word))
            {
                if out.iter().any(|s| s.text == cand) {
                    continue;
                }
                if let Some(m_lang) = detect_lang(&cand) {
                    if self.spell.is_correct(&cand, m_lang) {
                        out.push(Suggestion::new(cand, SuggestionKind::SpellFix, word));
                    }
                }
            }
            // Remaining context suggestions (weaker than the strong-fix path).
            for f in &ctx_fixes {
                if f.is_ok || f.confidence < 0.5 {
                    continue;
                }
                if !out.iter().any(|s| s.text == f.text) {
                    out.push(Suggestion::new(f.text.clone(), SuggestionKind::SpellFix, word));
                }
            }
        } else {
            // Compat path: the old cheap neighbors only.
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
        }

        // Drop far-away junk that only confuses ranking / auto-replace.
        let out = filter_candidates(word, out, 3);
        Some(out)
    }

    fn layout_candidate(&self, word: &str) -> Option<Suggestion> {
        let conv = {
            let tables = self.tables.read().unwrap();
            convert(word, &tables, None, None)
        };
        let conv = conv.ok()?;
        let converted = conv.converted;
        if converted == word {
            return None;
        }
        let c_lang = detect_lang(&converted)?;
        if self.spell.is_correct(&converted, c_lang) {
            Some(Suggestion::new(
                converted,
                SuggestionKind::LayoutConvert,
                word,
            ))
        } else {
            None
        }
    }

    /// Ask the hybrid backend about `word`. Soft-fails to empty.
    async fn context_fixes(
        &self,
        word: &str,
        lang: &str,
        cfg: &AppConfig,
    ) -> Vec<crate::ports::ContextFix> {
        let Some(ctx) = self.context.lock().unwrap().clone() else {
            return Vec::new();
        };
        let tail = self.buffer.peek_tail(cfg.spell.context_chars);
        // Context should not end with the word itself twice.
        let context = {
            let t = tail.trim_end();
            t.strip_suffix(word)
                .map(|s| s.trim_end().to_string())
                .unwrap_or_else(|| t.to_string())
        };
        match ctx.correct(word, &context, lang).await {
            Ok(fixes) => {
                if !fixes.is_empty() {
                    debug!(
                        "context[{}] word={:?} fixes={:?}",
                        ctx.name(),
                        word,
                        fixes
                            .iter()
                            .map(|f| (f.text.as_str(), f.is_ok, f.confidence))
                            .collect::<Vec<_>>()
                    );
                }
                fixes
            }
            Err(e) => {
                debug!("context[{}] failed: {e}", ctx.name());
                Vec::new()
            }
        }
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

        // Ignore our own injected keystrokes so they don't re-complete the word.
        self.suppress.store(true, Ordering::SeqCst);

        // The completing key (Space) may still be held — we must wait until
        // it is released, or the app ignores / reorders our backspace and
        // the typed separator is lost.
        self.buffer.wait_keys_quiet(50, 500).await;

        // Fully deterministic: delete word + the separator after it, then
        // type the replacement and put the separator back. Never touch the
        // space *before* the word (it is outside this range).
        let sep_len = trailing.chars().count();
        let total = erase_len + sep_len;
        if total > 0 {
            if let Err(e) = self.injector.backspace(total).await {
                tracing::warn!("spell replace failed (backspace {total}): {e}");
                self.suppress.store(false, Ordering::SeqCst);
                return SpellAction::None;
            }
        }
        if let Err(e) = self.injector.type_text(replacement, &layout).await {
            tracing::warn!("spell replace failed (type {:?}): {e}", replacement);
            self.suppress.store(false, Ordering::SeqCst);
            return SpellAction::None;
        }
        // Always restore the separator so the caret ends up *after* it —
        // the user must not press Space a second time.
        if !trailing.is_empty() {
            let sep_layout = switchfix_domain::LayoutId::new("us");
            if let Err(e) = self.injector.type_text(trailing, &sep_layout).await {
                tracing::warn!("spell replace failed (type separator {trailing:?}): {e}");
            }
            // Let the injected key commit before we return (some apps
            // coalesce uinput events if we immediately continue).
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // Sync buffer to what is now on screen — without word events.
        let mut synced = replacement.to_string();
        synced.push_str(trailing);
        self.buffer.set_contents(&synced);

        self.suppress.store(false, Ordering::SeqCst);
        // One auto-replace per word: wait for new typing before the next.
        self.auto_block.store(true, Ordering::SeqCst);
        *self.last_replace.lock().unwrap() = Some(Instant::now());
        // last_word is now the replacement (Ctrl+Alt+F shouldn't undo us).
        let char_len = replacement.chars().count();
        *self.last_word.lock().unwrap() = Some((
            replacement.to_string(),
            char_len,
            trailing.chars().next().unwrap_or(' '),
        ));

        debug!(
            from = original,
            to = replacement,
            trailing,
            "spell replaced (word+sep rewritten; space before untouched)"
        );
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
        async fn cursor_left(&self, count: usize) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("left:{count}"));
            Ok(())
        }
        async fn cursor_right(&self, count: usize) -> Result<(), InjectError> {
            self.log.lock().unwrap().push(format!("right:{count}"));
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
        // Delete word + the trailing space, type replacement + space.
        // Space *before* the word is outside the backspace range.
        assert_eq!(log, vec!["bs:8", "type:привет:ru", "type: :us"]);
    }

    #[tokio::test]
    async fn replace_preserves_spaces_around_word() {
        let (svc, i, _p) = setup("auto");
        // «приветт» + Space → replace word, keep space before, restore space after.
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "приветт".into(),
                    char_len: 7,
                },
                separator: ' ',
            })
            .await;
        assert!(matches!(action, SpellAction::Replaced { .. }));
        let log = i.log.lock().unwrap().clone();
        assert_eq!(log[0], "bs:8", "word (7) + trailing space (1)");
        assert_eq!(log[1], "type:привет:ru");
        assert_eq!(log[2], "type: :us", "space after must be typed");
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
        assert_eq!(log[0], "bs:7");
        assert_eq!(log[1], "type:привет:ru");
        assert_eq!(log[2], "type: :us");
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
        assert_eq!(log[1], "type:привет:ru");
        assert_eq!(log[2], "type: :us");
    }

    #[tokio::test]
    async fn hint_mode_never_auto_replaces() {
        let (svc, i, _p) = setup("hint");
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "приветт".into(),
                    char_len: 7,
                },
                separator: ' ',
            })
            .await;
        // Hints only — no inject.
        assert_eq!(action, SpellAction::ShowedHints);
        assert!(i.log.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn auto_mode_blocks_second_replace_in_a_row() {
        let (svc, i, _p) = setup("auto");
        let ev = || WordEvent::Completed {
            word: WordSpan {
                text: "приветт".into(),
                char_len: 7,
            },
            separator: ' ',
        };
        let a1 = svc.on_word_event(ev()).await;
        assert!(matches!(a1, SpellAction::Replaced { .. }));
        // Second Space on the same word must NOT replace again.
        let a2 = svc.on_word_event(ev()).await;
        assert_ne!(a2, SpellAction::Replaced { from: "приветт".into(), to: "привет".into() });
        let log = i.log.lock().unwrap().clone();
        // Exactly one backspace+type sequence from the first replace.
        assert_eq!(log.iter().filter(|l| l.starts_with("bs:")).count(), 1);

        // User types a new word → auto is re-armed.
        svc.on_word_event(WordEvent::Updated("приветт".into())).await;
        let a3 = svc.on_word_event(ev()).await;
        assert!(matches!(a3, SpellAction::Replaced { .. }));
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

    struct FakeContext {
        fixes: StdMutex<Vec<crate::ports::ContextFix>>,
        calls: StdMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl crate::ports::ContextCorrector for FakeContext {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn correct(
            &self,
            word: &str,
            _context: &str,
            _lang: &str,
        ) -> Result<Vec<crate::ports::ContextFix>, SpellError> {
            self.calls.lock().unwrap().push(word.to_string());
            Ok(self.fixes.lock().unwrap().clone())
        }
    }

    fn setup_with_context(
        mode: &str,
        ctx: Arc<FakeContext>,
    ) -> (SpellService, Arc<FakeInjector>, Arc<FakePresenter>) {
        let (svc, i, p) = setup(mode);
        svc.set_context_corrector(ctx);
        (svc, i, p)
    }

    #[tokio::test]
    async fn hybrid_neighbors_find_typo_dict_missed() {
        // FakeSpell.suggest("helo") is empty — only edit-1 neighbors can save us.
        let (svc, _i, p) = setup("hint");
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "helo".into(),
                    char_len: 4,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(action, SpellAction::ShowedHints, "edit-1 should find hello");
        assert!(p.is_visible());
        let action = svc.accept_slot(1).await;
        assert_eq!(
            action,
            SpellAction::Replaced {
                from: "helo".into(),
                to: "hello".into(),
            }
        );
    }

    #[tokio::test]
    async fn context_veto_suppresses_dict_false_positive() {
        // Dict says «запустил» is wrong (inflection gap) — context says it's fine.
        let ctx = Arc::new(FakeContext {
            fixes: StdMutex::new(vec![crate::ports::ContextFix {
                text: "запустил".into(),
                confidence: 0.9,
                is_ok: true,
            }]),
            calls: StdMutex::new(Vec::new()),
        });
        let (svc, _i, p) = setup_with_context("hint", ctx.clone());
        let action = svc
            .on_word_event(WordEvent::Completed {
                word: WordSpan {
                    text: "запустил".into(),
                    char_len: 8,
                },
                separator: ' ',
            })
            .await;
        assert_eq!(action, SpellAction::None);
        assert!(!p.is_visible());
        assert_eq!(ctx.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn context_strong_fix_wins_over_dict_pile() {
        let ctx = Arc::new(FakeContext {
            fixes: StdMutex::new(vec![crate::ports::ContextFix {
                text: "привет".into(),
                confidence: 0.92,
                is_ok: false,
            }]),
            calls: StdMutex::new(Vec::new()),
        });
        let (svc, _i, _p) = setup_with_context("hint", ctx);
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
        let action = svc.accept_slot(1).await;
        assert_eq!(
            action,
            SpellAction::Replaced {
                from: "приветт".into(),
                to: "привет".into(),
            }
        );
    }

    // Silence unused import warnings for re-exports used by fakes.
    #[allow(dead_code)]
    fn _unused(_: LayoutError, _: LayoutIndex, _: LayoutInfo, _: SavedClipboard) {}
}
