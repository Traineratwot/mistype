//! Language detection by script (used for undo `type_text` layout choice).

/// Detect the language for a word by script.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_lang_by_script() {
        assert_eq!(detect_lang("привет"), Some("ru"));
        assert_eq!(detect_lang("hello"), Some("en"));
        assert_eq!(detect_lang("hello мир"), Some("en")); // majority latin (5 > 3)
        assert_eq!(detect_lang("мир hello"), Some("en")); // still latin majority
        assert_eq!(detect_lang("привет world"), Some("ru")); // cyrillic 6 > latin 5
        assert_eq!(detect_lang("123!"), None);
        assert_eq!(detect_lang(""), None);
    }
}
