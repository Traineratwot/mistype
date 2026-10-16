//! `SpellChecker` backed by zspell (pure Rust, Hunspell-compatible).
//!
//! Dictionaries: `/usr/share/hunspell/{lang}.aff` + `{lang}.dic`
//! (e.g. `ru_RU`, `en_US`) plus a personal word list at
//! `~/.local/share/switchfix/user.dic` (one word per line).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use switchfix_app::ports::{SpellChecker, SpellError};
use tracing::{info, warn};

/// One loaded language dictionary.
struct LangDict {
    lang: String,
    dict: zspell::Dictionary,
    /// Sorted stems used for word completion (base forms from `.dic` + user words).
    completion: Vec<String>,
}

/// Shared spell dictionary (zspell + personal word list).
pub struct ZspellDictionary {
    langs: Mutex<Vec<LangDict>>,
    user_dic_path: PathBuf,
    /// Learned words, kept so a rebuild keeps them.
    user_words: Mutex<BTreeSet<String>>,
}

impl ZspellDictionary {
    /// Load every requested language from the system hunspell dir.
    ///
    /// Missing dictionaries are skipped with a warning — spellcheck degrades
    /// to "always correct" for that language instead of erroring out.
    pub fn load(languages: &[String], user_dic_path: PathBuf) -> Arc<Self> {
        let user_words = read_user_words(&user_dic_path);
        let personal = user_words
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        let mut langs = Vec::new();
        for lang in languages {
            match load_lang(lang, &personal, &user_words) {
                Ok(ld) => {
                    info!(lang = %ld.lang, "spell dictionary loaded ({} stems)", ld.completion.len());
                    langs.push(ld);
                }
                Err(e) => {
                    warn!(lang, error = %e, "spell dictionary not loaded");
                }
            }
        }

        Arc::new(Self {
            langs: Mutex::new(langs),
            user_dic_path,
            user_words: Mutex::new(user_words),
        })
    }

    /// `true` when at least one language dictionary is available.
    pub fn has_any(&self) -> bool {
        !self.langs.lock().unwrap().is_empty()
    }

    fn with_lang<R>(&self, lang: &str, f: impl FnOnce(&LangDict) -> R) -> Option<R> {
        let langs = self.langs.lock().unwrap();
        langs.iter().find(|l| l.lang == lang).map(f)
    }

    /// Rebuild all language dicts after the personal word list changed.
    #[allow(dead_code)]
    fn rebuild(&self, languages: &[String]) {
        let user_words = self.user_words.lock().unwrap().clone();
        let personal = user_words
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let mut langs = Vec::new();
        for lang in languages {
            if let Ok(ld) = load_lang(lang, &personal, &user_words) {
                langs.push(ld);
            }
        }
        *self.langs.lock().unwrap() = langs;
    }
}

fn load_lang(lang: &str, personal: &str, user_words: &BTreeSet<String>) -> Result<LangDict, String> {
    let (aff, dic) = system_dict_paths(lang).ok_or_else(|| {
        format!("no hunspell files for language '{lang}' in /usr/share/hunspell")
    })?;
    let aff_s = std::fs::read_to_string(&aff).map_err(|e| format!("{}: {e}", aff.display()))?;
    let dic_s = std::fs::read_to_string(&dic).map_err(|e| format!("{}: {e}", dic.display()))?;

    let mut builder = zspell::DictBuilder::new()
        .config_str(&aff_s)
        .dict_str(&dic_s);
    if !personal.is_empty() {
        builder = builder.personal_str(personal);
    }
    let dict = builder.build().map_err(|e| format!("zspell build: {e}"))?;

    let mut completion: Vec<String> = dic_stems(&dic_s);
    for w in user_words {
        completion.push(w.clone());
    }
    completion.sort();
    completion.dedup();

    Ok(LangDict {
        lang: lang.to_string(),
        dict,
        completion,
    })
}

/// Map short lang id (`ru`, `en`) to system hunspell file stems.
fn system_dict_paths(lang: &str) -> Option<(PathBuf, PathBuf)> {
    let candidates: &[&str] = match lang {
        "ru" => &["ru_RU", "ru"],
        "en" => &["en_US", "en_GB", "en"],
        "uk" => &["uk_UA", "uk"],
        "de" => &["de_DE", "de"],
        _ => &[], // fall through to direct lookup below
    };
    let dir = Path::new("/usr/share/hunspell");
    if !candidates.is_empty() {
        for stem in candidates {
            let aff = dir.join(format!("{stem}.aff"));
            let dic = dir.join(format!("{stem}.dic"));
            if aff.is_file() && dic.is_file() {
                return Some((aff, dic));
            }
        }
        return None;
    }
    // Unknown lang id: try it as a file stem as-is.
    let aff = dir.join(format!("{lang}.aff"));
    let dic = dir.join(format!("{lang}.dic"));
    if aff.is_file() && dic.is_file() {
        Some((aff, dic))
    } else {
        None
    }
}

/// First column of a hunspell `.dic` (the stem word).
fn dic_stems(dic_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (i, line) in dic_text.lines().enumerate() {
        if i == 0 && line.trim().parse::<usize>().is_ok() {
            continue; // word-count header
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let stem = line.split('/').next().unwrap_or(line).trim();
        // Strip morphological fields (`po:noun` etc.).
        let stem = stem.split_whitespace().next().unwrap_or(stem);
        if !stem.is_empty() {
            out.push(stem.to_string());
        }
    }
    out
}

fn read_user_words(path: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let w = line.trim();
            if w.is_empty() || w.starts_with('#') || w.starts_with('*') {
                continue;
            }
            set.insert(w.to_string());
        }
    }
    set
}

fn append_user_word(path: &Path, word: &str) -> Result<(), SpellError> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| SpellError::Failed(format!("create {}: {e}", parent.display())))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| SpellError::Failed(format!("open {}: {e}", path.display())))?;
    writeln!(f, "{word}")
        .map_err(|e| SpellError::Failed(format!("write user.dic: {e}")))
}

#[async_trait]
impl SpellChecker for ZspellDictionary {
    fn is_correct(&self, word: &str, lang: &str) -> bool {
        // Personal words always pass.
        if self.user_words.lock().unwrap().contains(word) {
            return true;
        }
        match self.with_lang(lang, |l| l.dict.check_word(word)) {
            Some(ok) => ok,
            // No dictionary for this language — never block / never spam.
            None => true,
        }
    }

    fn suggest(&self, word: &str, lang: &str, n: usize) -> Vec<String> {
        let Some(list) = self.with_lang(lang, |l| {
            l.dict
                .entry(word)
                .suggest()
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.to_string())
                .take(n)
                .collect::<Vec<_>>()
        }) else {
            return Vec::new();
        };
        list
    }

    fn complete(&self, prefix: &str, lang: &str, n: usize) -> Vec<String> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let lower = prefix.to_lowercase();
        let Some(list) = self.with_lang(lang, |l| {
            // Binary-search the first candidate with this prefix.
            let start = l
                .completion
                .partition_point(|w| w.to_lowercase() < lower);
            let mut out = Vec::new();
            for w in &l.completion[start..] {
                if out.len() >= n {
                    break;
                }
                if !w.to_lowercase().starts_with(&lower) {
                    break;
                }
                if w.chars().count() > prefix.chars().count() {
                    out.push(w.clone());
                }
            }
            out
        }) else {
            return Vec::new();
        };
        list
    }

    fn add_to_dictionary(&self, word: &str) -> Result<(), SpellError> {
        let w = word.trim();
        if w.is_empty() {
            return Err(SpellError::Failed("empty word".into()));
        }
        {
            let mut set = self.user_words.lock().unwrap();
            if !set.insert(w.to_string()) {
                return Ok(()); // already present
            }
        }
        append_user_word(&self.user_dic_path, w)?;

        // Cheap in-memory bump for completion lists; full rebuild is not needed
        // for correctness of `is_correct` (that checks `user_words` first).
        let mut langs = self.langs.lock().unwrap();
        for l in langs.iter_mut() {
            if !l.completion.iter().any(|c| c == w) {
                l.completion.push(w.to_string());
                l.completion.sort();
            }
        }
        Ok(())
    }

    fn languages(&self) -> Vec<String> {
        self.langs.lock().unwrap().iter().map(|l| l.lang.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dict() -> Arc<ZspellDictionary> {
        let tmp = std::env::temp_dir().join(format!("switchfix-spell-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        ZspellDictionary::load(
            &["en".into(), "ru".into()],
            tmp.join("user.dic"),
        )
    }

    #[test]
    fn loads_system_dictionaries_if_present() {
        let d = test_dict();
        // On a Kubuntu box both should load; if not, just skip assertions.
        if d.languages().contains(&"en".to_string()) {
            assert!(d.is_correct("hello", "en"));
            assert!(!d.is_correct("helo", "en"));
        }
        if d.languages().contains(&"ru".to_string()) {
            assert!(d.is_correct("привет", "ru"));
        }
    }

    #[test]
    fn missing_lang_is_always_correct() {
        let d = test_dict();
        assert!(d.is_correct("whatever", "xx"));
        assert!(d.suggest("whatever", "xx", 3).is_empty());
    }

    #[test]
    fn suggest_returns_candidates_for_typo() {
        let d = test_dict();
        if !d.languages().contains(&"en".to_string()) {
            return;
        }
        let s = d.suggest("helo", "en", 10);
        assert!(!s.is_empty(), "expected candidates for 'helo'");
        // zspell's own ranking may not put `hello` first; domain `rank()` reorders.
        assert!(
            s.iter().any(|c| c == "hello" || c == "hell" || c == "halo"),
            "got {s:?}"
        );
    }

    #[test]
    fn complete_prefix() {
        let d = test_dict();
        if !d.languages().contains(&"en".to_string()) {
            return;
        }
        let c = d.complete("hel", "en", 5);
        assert!(c.iter().any(|w| w.starts_with("hel")), "got {c:?}");
        assert!(c.iter().all(|w| w.chars().count() > 3));
    }

    #[test]
    fn user_dic_learned_word_is_correct() {
        let tmp = std::env::temp_dir().join(format!("switchfix-userdic-{}", std::process::id()));
        let _ = std::fs::remove_file(tmp.join("user.dic"));
        let d = ZspellDictionary::load(&["en".into()], tmp.join("user.dic"));
        let word = "zzznotawordx";
        if d.languages().contains(&"en".to_string()) {
            assert!(!d.is_correct(word, "en"), "should start misspelled");
        }
        d.add_to_dictionary(word).unwrap();
        assert!(d.is_correct(word, "en"));
        // Survives a reload.
        let d2 = ZspellDictionary::load(&["en".into()], tmp.join("user.dic"));
        assert!(d2.is_correct(word, "en"));
    }

    #[test]
    fn dic_stems_skips_header_and_flags() {
        let text = "3\nhello\nworld/MS\nfoo po:noun\n# comment\n";
        assert_eq!(dic_stems(text), vec!["hello", "world", "foo"]);
    }
}
