//! Pure spell/typo heuristics — no I/O, no dictionary engine.
//!
//! Ranking and filtering of candidate corrections. The dictionary lookup
//! itself lives in adapters (`SpellChecker` port).

use serde::{Deserialize, Serialize};

/// Why a candidate is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionKind {
    /// Word completion while the user is still typing.
    Completion,
    /// Spelling correction for a finished word.
    SpellFix,
    /// Same word after EN↔RU layout conversion.
    LayoutConvert,
}

/// One candidate shown in the suggestion bar / applied on hotkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
    /// The original word the candidate replaces.
    pub source_word: String,
    /// Which hybrid layer produced this candidate:
    /// `"layout"` | `"dict"` | `"edit1"` | `"context"` (LLM / LanguageTool).
    #[serde(default)]
    pub via: String,
}

impl Suggestion {
    pub fn new(text: impl Into<String>, kind: SuggestionKind, source_word: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
            source_word: source_word.into(),
            via: String::new(),
        }
    }

    pub fn with_via(mut self, via: impl Into<String>) -> Self {
        self.via = via.into();
        self
    }
}

/// Should we run a dictionary check on this word?
///
/// Skips noise: too short, digits-only, ALL-CAPS acronyms, URL-ish tokens.
pub fn should_check(word: &str, min_len: usize) -> bool {
    let w = word.trim();
    if w.chars().count() < min_len {
        return false;
    }
    if w.chars().all(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == ':' || c == '/') {
        return false;
    }
    // ALLCAPS acronyms (АБВ, NASA, HTML) — not typos.
    let letters: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase()) {
        return false;
    }
    // URL / email / path fragments.
    if w.contains("://") || w.contains('@') || w.contains('/') || w.contains('\\') {
        return false;
    }
    // Mixed with digits like `utf8`, `mp3` — leave alone.
    if w.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    letters.iter().any(|c| c.is_alphabetic())
}

/// Detect the dictionary language for a word by script.
///
/// Cyrillic → `"ru"`, Latin → `"en"`, otherwise `None`.
pub fn detect_lang(word: &str) -> Option<&'static str> {
    let mut cyr = 0usize;
    let mut lat = 0usize;
    for c in word.chars() {
        if c.is_alphabetic() {
            if matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё') {
                cyr += 1;
            } else if c.is_ascii_alphabetic() {
                lat += 1;
            }
        }
    }
    if cyr == 0 && lat == 0 {
        None
    } else if cyr >= lat {
        Some("ru")
    } else {
        Some("en")
    }
}

/// Cheap one-edit neighbors used to recover obvious typos the engine missed.
pub fn one_step_neighbors(word: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 3 {
        return out;
    }
    // Drop last / first char.
    out.push(chars[..chars.len() - 1].iter().collect());
    out.push(chars[1..].iter().collect());
    // Swap the last two chars (`teh` → `the` style, tail-only to stay cheap).
    {
        let mut s = chars.clone();
        let n = s.len();
        s.swap(n - 1, n - 2);
        out.push(s.into_iter().collect());
    }
    // Double-letter collapse (`приветт` → `привет`).
    if chars.len() >= 2 && chars[chars.len() - 1] == chars[chars.len() - 2] {
        out.push(chars[..chars.len() - 1].iter().collect());
    }
    out
}

/// Longest word for which we generate the full edit-1 neighborhood.
const MAX_EDIT1_WORD: usize = 12;

/// Alphabet used for insert/replace candidates (by script).
fn alphabet_for(word: &str) -> &'static [char] {
    match detect_lang(word) {
        Some("ru") => &[
            'а', 'б', 'в', 'г', 'д', 'е', 'ё', 'ж', 'з', 'и', 'й', 'к', 'л', 'м', 'н', 'о', 'п',
            'р', 'с', 'т', 'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ъ', 'ы', 'ь', 'э', 'ю', 'я',
        ],
        _ => &[
            'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q',
            'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
        ],
    }
}

/// Full edit-1 neighborhood: deletes, transposes, replaces, inserts.
///
/// Filter with the dictionary (`is_correct`) on the caller side — this is
/// deliberately over-generating so we stop missing obvious typos. Cost is
/// O(len × alphabet); capped at [`MAX_EDIT1_WORD`].
pub fn edit1_neighbors(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    if n < 3 {
        return one_step_neighbors(word);
    }
    // Long words: keep the cheap set (full edit-1 explodes).
    if n > MAX_EDIT1_WORD {
        return one_step_neighbors(word);
    }

    let mut out = Vec::with_capacity(n * 40);
    let mut push = |s: String| {
        if !out.contains(&s) {
            out.push(s);
        }
    };

    // Deletes.
    for i in 0..n {
        let mut s = String::with_capacity(word.len());
        s.extend(chars[..i].iter());
        s.extend(chars[i + 1..].iter());
        push(s);
    }
    // Transposes.
    for i in 0..n.saturating_sub(1) {
        let mut s = chars.clone();
        s.swap(i, i + 1);
        push(s.into_iter().collect());
    }
    // Replaces + inserts with the language alphabet.
    let alpha = alphabet_for(word);
    for i in 0..n {
        for &c in alpha {
            if c == chars[i] {
                continue;
            }
            let mut s = String::with_capacity(word.len());
            s.extend(chars[..i].iter());
            s.push(c);
            s.extend(chars[i + 1..].iter());
            push(s);
        }
    }
    for i in 0..=n {
        for &c in alpha {
            let mut s = String::with_capacity(word.len());
            s.extend(chars[..i].iter());
            s.push(c);
            s.extend(chars[i..].iter());
            push(s);
        }
    }

    // Double-letter collapse at every position (`приветт`, `hhello`).
    for i in 1..n {
        if chars[i] == chars[i - 1] {
            let mut s = String::with_capacity(word.len());
            s.extend(chars[..i].iter());
            s.extend(chars[i + 1..].iter());
            push(s);
        }
    }
    out
}

/// Common confusable pairs (keyboard / phonetic) — much smaller than edit-1.
///
/// Applied one position at a time; callers still filter via the dictionary.
pub fn confusable_neighbors(word: &str) -> Vec<String> {
    const EN_PAIRS: &[(char, char)] = &[
        ('e', 'i'),
        ('i', 'e'),
        ('a', 'e'),
        ('o', 'a'),
        ('s', 'c'),
        ('c', 's'),
        ('k', 'c'),
        ('m', 'n'),
        ('n', 'm'),
        ('t', 'd'),
        ('d', 't'),
        ('l', 'r'),
        ('r', 'l'),
        ('w', 'v'),
        ('v', 'w'),
        ('b', 'p'),
        ('p', 'b'),
        ('g', 'j'),
    ];
    const RU_PAIRS: &[(char, char)] = &[
        ('е', 'и'),
        ('и', 'е'),
        ('ё', 'о'),
        ('о', 'ё'),
        ('ы', 'и'),
        ('и', 'ы'),
        ('ь', 'ъ'),
        ('ъ', 'ь'),
        ('т', 'д'),
        ('д', 'т'),
        ('з', 'с'),
        ('с', 'з'),
        ('ж', 'ш'),
        ('ш', 'ж'),
        ('ч', 'щ'),
        ('щ', 'ч'),
        ('л', 'р'),
        ('р', 'л'),
        ('м', 'н'),
        ('н', 'м'),
        ('в', 'ф'),
        ('ф', 'в'),
        ('к', 'х'),
        ('г', 'к'),
        ('б', 'п'),
        ('п', 'б'),
        ('у', 'ю'),
        ('ю', 'у'),
    ];
    let pairs: &[(char, char)] = match detect_lang(word) {
        Some("ru") => RU_PAIRS,
        _ => EN_PAIRS,
    };
    let chars: Vec<char> = word.chars().collect();
    let mut out = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        for &(a, b) in pairs {
            let repl = if c == a {
                Some(b)
            } else if c == b {
                Some(a)
            } else {
                None
            };
            if let Some(r) = repl {
                if r == c {
                    continue;
                }
                let mut s = String::with_capacity(word.len());
                s.extend(chars[..i].iter());
                s.push(r);
                s.extend(chars[i + 1..].iter());
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// How strongly we trust `s` as a replacement for `word` (0.0..=1.0).
///
/// Used by auto-replace: only high-confidence fixes are applied silently.
pub fn suggestion_confidence(word: &str, s: &Suggestion) -> f32 {
    match s.kind {
        SuggestionKind::LayoutConvert => 0.95,
        // Context (LT / LLM) ranked this — treat as strong but not absolute.
        SuggestionKind::SpellFix if s.text.chars().count() > 0 && word != s.text => {
            let dist = edit_distance(word, &s.text);
            match dist {
                0 => 0.0,
                1 => 0.82,
                2 => 0.55,
                _ => 0.25,
            }
        }
        SuggestionKind::SpellFix => 0.0,
        SuggestionKind::Completion => 0.35,
    }
}

/// Drop weak far-away candidates that only add noise to the bar.
pub fn filter_candidates(word: &str, candidates: Vec<Suggestion>, max_dist: usize) -> Vec<Suggestion> {
    candidates
        .into_iter()
        .filter(|s| {
            if s.text == word {
                return false;
            }
            if s.kind == SuggestionKind::LayoutConvert {
                return true;
            }
            if s.kind == SuggestionKind::Completion {
                return true;
            }
            edit_distance(word, &s.text) <= max_dist
        })
        .collect()
}

/// Levenshtein distance (chars, not bytes).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Auto-replace only when the fix is obvious.
///
/// Exactly one strong candidate, close to the original, word long enough.
pub fn is_auto_fixable(suggestions: &[Suggestion], word: &str, min_len: usize) -> bool {
    if suggestions.len() != 1 {
        return false;
    }
    if word.chars().count() < min_len {
        return false;
    }
    let dist = edit_distance(word, &suggestions[0].text);
    dist >= 1 && dist <= 2
}

/// Should we silently replace `word` with `suggestions[0]` right after the
/// user finishes the word (space / punctuation)?
///
/// Stricter about confidence, looser about "only one candidate" than
/// [`is_auto_fixable`]: layout-convert always wins; a single close edit wins;
/// with several candidates the top one must be clearly closer (or distance 1).
pub fn should_auto_apply(suggestions: &[Suggestion], word: &str, min_len: usize) -> bool {
    let Some(top) = suggestions.first() else {
        return false;
    };
    if top.text == word {
        return false;
    }
    // Wrong-layout word (`ghbdtn` → `привет`) is high confidence.
    if top.kind == SuggestionKind::LayoutConvert {
        return true;
    }
    if word.chars().count() < min_len {
        return false;
    }
    let dist = edit_distance(word, &top.text);
    if !(1..=2).contains(&dist) {
        return false;
    }
    // Confidence gate: hybrid / context fixes still need to be close.
    if suggestion_confidence(word, top) < 0.75 {
        return false;
    }
    if suggestions.len() > 1 {
        let second = edit_distance(word, &suggestions[1].text);
        // Prefer an obvious one-key typo over a tie.
        return dist == 1 || dist < second;
    }
    true
}

/// Order candidates for the suggestion bar.
///
/// Layout-convert first (SwitchFix core), then closer edits, then shorter words
/// (usually the more common form). Stable within the same rank.
pub fn rank(word: &str, mut candidates: Vec<Suggestion>) -> Vec<Suggestion> {
    candidates.sort_by(|x, y| {
        kind_key(x.kind)
            .cmp(&kind_key(y.kind))
            .then_with(|| edit_distance(word, &x.text).cmp(&edit_distance(word, &y.text)))
            .then_with(|| x.text.chars().count().cmp(&y.text.chars().count()))
            .then_with(|| x.text.cmp(&y.text))
    });
    candidates.dedup_by(|a, b| a.text == b.text);
    candidates
}

fn kind_key(k: SuggestionKind) -> u8 {
    match k {
        SuggestionKind::LayoutConvert => 0,
        SuggestionKind::SpellFix => 1,
        SuggestionKind::Completion => 2,
    }
}

/// Apply the original word's casing pattern to the replacement.
///
/// `Hello` + `world` → `World`; `HELLO` + `world` → `WORLD`; `hello` + `World` → `world`.
pub fn preserve_case(original: &str, replacement: &str) -> String {
    let orig_chars: Vec<char> = original.chars().collect();
    if orig_chars.is_empty() {
        return replacement.to_string();
    }
    let all_upper = orig_chars.iter().filter(|c| c.is_alphabetic()).all(|c| c.is_uppercase())
        && orig_chars.iter().any(|c| c.is_alphabetic());
    if all_upper {
        return replacement.to_uppercase();
    }
    let first_alpha_upper = orig_chars
        .iter()
        .find(|c| c.is_alphabetic())
        .map(|c| c.is_uppercase())
        .unwrap_or(false);
    if first_alpha_upper {
        // Title-case the first alphabetic char of the replacement.
        let mut out = String::with_capacity(replacement.len());
        let mut seen_alpha = false;
        for c in replacement.chars() {
            if !seen_alpha && c.is_alphabetic() {
                out.extend(c.to_uppercase());
                seen_alpha = true;
            } else {
                out.push(c);
            }
        }
        out
    } else {
        // Original is lowercase — force replacement lowercase.
        replacement.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_check_skips_short_and_noise() {
        assert!(!should_check("ab", 3));
        assert!(should_check("abc", 3));
        assert!(!should_check("123", 3));
        assert!(!should_check("HTML", 3));
        assert!(!should_check("https://x.io", 3));
        assert!(!should_check("user@mail", 3));
        assert!(!should_check("utf8", 3));
        assert!(should_check("привет", 3));
        assert!(should_check("hello", 3));
    }

    #[test]
    fn edit_distance_basic() {
        assert_eq!(edit_distance("hello", "hello"), 0);
        assert_eq!(edit_distance("hello", "hallo"), 1);
        assert_eq!(edit_distance("привет", "првет"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn auto_fixable_requires_single_close_candidate() {
        let s = |t: &str| Suggestion::new(t, SuggestionKind::SpellFix, "helo");
        assert!(is_auto_fixable(&[s("hello")], "helo", 4));
        assert!(!is_auto_fixable(&[s("hello"), s("hallo")], "helo", 4));
        assert!(!is_auto_fixable(&[s("hello")], "helo", 5)); // too short
        assert!(!is_auto_fixable(&[s("hello")], "hello", 4)); // dist 0 = already correct
        assert!(!is_auto_fixable(
            &[Suggestion::new("hello", SuggestionKind::SpellFix, "he")],
            "he",
            4
        ));
    }

    #[test]
    fn should_auto_apply_takes_close_top_even_with_competing_candidates() {
        let spell = |t: &str| Suggestion::new(t, SuggestionKind::SpellFix, "приветт");
        // One-key typo wins even if other candidates exist.
        assert!(should_auto_apply(
            &[spell("привет"), spell("привету"), spell("приветы")],
            "приветт",
            4
        ));
        // Ambiguous two-edit tie does not.
        assert!(!should_auto_apply(
            &[spell("abcde"), spell("abcdf")],
            "abcxx",
            4
        ));
        // Layout convert always applies.
        let lc = Suggestion::new("привет", SuggestionKind::LayoutConvert, "ghbdtn");
        assert!(should_auto_apply(&[lc], "ghbdtn", 4));
        // Too-short spell fixes stay conservative.
        assert!(!should_auto_apply(&[spell("ab")], "ax", 4));
    }

    #[test]
    fn rank_puts_layout_convert_first() {
        let word = "ghbdtn";
        let mut c = vec![
            Suggestion::new("ghbdtnm", SuggestionKind::SpellFix, word),
            Suggestion::new("привет", SuggestionKind::LayoutConvert, word),
        ];
        c = rank(word, c);
        assert_eq!(c[0].kind, SuggestionKind::LayoutConvert);
        assert_eq!(c[0].text, "привет");
    }

    #[test]
    fn rank_dedups_and_prefers_closer() {
        let word = "helo";
        let c = rank(
            word,
            vec![
                Suggestion::new("help", SuggestionKind::SpellFix, word),
                Suggestion::new("hello", SuggestionKind::SpellFix, word),
                Suggestion::new("hello", SuggestionKind::SpellFix, word),
            ],
        );
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].text, "help"); // dist 1 < dist 2
    }

    #[test]
    fn one_step_neighbors_covers_common_typos() {
        let n = one_step_neighbors("приветт");
        assert!(n.contains(&"привет".to_string()), "drop last: {n:?}");
        let n = one_step_neighbors("teh");
        assert!(n.contains(&"the".to_string()), "swap tail: {n:?}");
    }

    #[test]
    fn edit1_neighbors_includes_insert_delete_replace() {
        let n = edit1_neighbors("helo");
        assert!(n.contains(&"hello".to_string()), "insert: {n:?}");
        assert!(n.contains(&"elo".to_string()), "delete: {n:?}");
        assert!(n.contains(&"help".to_string()) || n.contains(&"held".to_string()), "replace");
        // Double-letter collapse still present.
        let n = edit1_neighbors("приветт");
        assert!(n.contains(&"привет".to_string()), "collapse: {n:?}");
    }

    #[test]
    fn edit1_long_word_falls_back_to_cheap() {
        let word = "оченьдлинноеслово";
        let n = edit1_neighbors(word);
        assert!(n.len() < 32, "cheap path for long words, got {}", n.len());
    }

    #[test]
    fn confusable_neighbors_covers_homophones() {
        let n = confusable_neighbors("привед");
        assert!(n.contains(&"привид".to_string()), "е→и: {n:?}");
        assert!(n.contains(&"превед".to_string()), "и→е: {n:?}");
        assert!(n.contains(&"привет".to_string()), "д→т: {n:?}");
    }

    #[test]
    fn filter_candidates_drops_far_and_self() {
        let w = "helo";
        let c = vec![
            Suggestion::new("hello", SuggestionKind::SpellFix, w),
            Suggestion::new("helo", SuggestionKind::SpellFix, w),
            Suggestion::new("elephant", SuggestionKind::SpellFix, w),
            Suggestion::new("привет", SuggestionKind::LayoutConvert, w),
        ];
        let f = filter_candidates(w, c, 2);
        assert!(f.iter().any(|s| s.text == "hello"));
        assert!(f.iter().any(|s| s.kind == SuggestionKind::LayoutConvert));
        assert!(!f.iter().any(|s| s.text == "helo"));
        assert!(!f.iter().any(|s| s.text == "elephant"));
    }

    #[test]
    fn detect_lang_by_script() {
        assert_eq!(detect_lang("привет"), Some("ru"));
        assert_eq!(detect_lang("hello"), Some("en"));
        assert_eq!(detect_lang("ghbdtn"), Some("en"));
        assert_eq!(detect_lang("123"), None);
        assert_eq!(detect_lang("мир"), Some("ru"));
    }

    #[test]
    fn preserve_case_patterns() {
        assert_eq!(preserve_case("hello", "World"), "world");
        assert_eq!(preserve_case("Hello", "world"), "World");
        assert_eq!(preserve_case("HELLO", "world"), "WORLD");
        assert_eq!(preserve_case("привет", "Здравствуйте"), "здравствуйте");
        assert_eq!(preserve_case("Привет", "здравствуйте"), "Здравствуйте");
    }
}
