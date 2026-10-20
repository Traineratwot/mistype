//! Character-level keyboard layout conversion (EN QWERTY ↔ RU ЙЦУКЕН).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::model::LayoutId;

/// Bidirectional map between two keyboard layouts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionTable {
    pub from: LayoutId,
    pub to: LayoutId,
    /// Character map `from_char -> to_char`. Reverse map is built on load.
    pub map: HashMap<char, char>,
}

impl ConversionTable {
    pub fn new(from: LayoutId, to: LayoutId, map: HashMap<char, char>) -> Self {
        Self { from, to, map }
    }

    pub fn reverse(&self) -> Self {
        let map = self.map.iter().map(|(k, v)| (*v, *k)).collect();
        Self {
            from: self.to.clone(),
            to: self.from.clone(),
            map,
        }
    }

    pub fn convert_char(&self, c: char) -> Option<char> {
        self.map.get(&c).copied()
    }
}

/// On-disk / custom map format (`assets/maps/*.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapFile {
    pub name: String,
    pub from: String,
    pub to: String,
    pub map: HashMap<char, char>,
}

impl MapFile {
    pub fn parse(json: &str) -> Result<Self, DomainError> {
        serde_json::from_str(json)
            .map_err(|e| DomainError::InvalidLayout(format!("map json: {e}")))
    }

    pub fn into_table(self) -> ConversionTable {
        ConversionTable::new(LayoutId::new(self.from), LayoutId::new(self.to), self.map)
    }
}

/// Set of tables covering the active layout pair(s).
#[derive(Debug, Clone, Default)]
pub struct ConversionSet {
    tables: Vec<ConversionTable>,
}

impl ConversionSet {
    pub fn new(tables: Vec<ConversionTable>) -> Self {
        Self { tables }
    }

    /// Build EN↔RU from the built-in QWERTY ↔ ЙЦУКЕН map.
    pub fn en_ru() -> Self {
        let forward = ConversionTable::new(
            LayoutId::new("us"),
            LayoutId::new("ru"),
            en_to_ru_map(),
        );
        Self::new(vec![forward.clone(), forward.reverse()])
    }

    pub fn find(&self, from: &LayoutId, to: &LayoutId) -> Option<&ConversionTable> {
        self.tables
            .iter()
            .find(|t| &t.from == from && &t.to == to)
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertedText {
    pub original: String,
    pub converted: String,
    pub from: LayoutId,
    pub to: LayoutId,
}

/// Detect likely layout of `text` by the share of Latin vs Cyrillic letters.
///
/// Returns `(detected, other)` or `None` if the text has no letters.
pub fn detect_direction(text: &str) -> Option<(LayoutId, LayoutId)> {
    let mut latin = 0usize;
    let mut cyrillic = 0usize;

    for c in text.chars() {
        if c.is_ascii_alphabetic() {
            latin += 1;
        } else if ('\u{0400}'..='\u{04FF}').contains(&c) {
            cyrillic += 1;
        }
    }

    match latin.cmp(&cyrillic) {
        std::cmp::Ordering::Greater => Some((LayoutId::new("us"), LayoutId::new("ru"))),
        std::cmp::Ordering::Less => Some((LayoutId::new("ru"), LayoutId::new("us"))),
        // Equal non-zero counts: treat as EN→RU (same as wordwrap convention).
        std::cmp::Ordering::Equal if latin > 0 => {
            Some((LayoutId::new("us"), LayoutId::new("ru")))
        }
        std::cmp::Ordering::Equal => None,
    }
}

/// Convert `text` between layouts.
///
/// - If `from`/`to` are `None`, direction is detected from the text.
/// - Characters without a mapping are kept unchanged.
/// - Returns `None` when there is nothing to convert (empty / unchanged / unknown direction).
pub fn convert(
    text: &str,
    tables: &ConversionSet,
    from: Option<&LayoutId>,
    to: Option<&LayoutId>,
) -> Result<ConvertedText, DomainError> {
    if text.is_empty() {
        return Err(DomainError::EmptyText);
    }

    let (src, dst) = match (from, to) {
        (Some(f), Some(t)) => (f.clone(), t.clone()),
        _ => {
            let (f, t) = detect_direction(text).ok_or(DomainError::UnknownDirection)?;
            (f, t)
        }
    };

    let table = tables
        .find(&src, &dst)
        .ok_or_else(|| DomainError::MissingTable {
            from: src.0.clone(),
            to: dst.0.clone(),
        })?;

    let converted: String = text.chars().map(|c| table.convert_char(c).unwrap_or(c)).collect();

    if converted == text {
        return Err(DomainError::NoInputFound);
    }

    Ok(ConvertedText {
        original: text.to_string(),
        converted,
        from: src,
        to: dst,
    })
}

/// Standard QWERTY → ЙЦУКЕН map (including Shift variants and a few symbols).
pub fn en_to_ru_map() -> HashMap<char, char> {
    let pairs: &[(char, char)] = &[
        // lowercase
        ('q', 'й'), ('w', 'ц'), ('e', 'у'), ('r', 'к'), ('t', 'е'),
        ('y', 'н'), ('u', 'г'), ('i', 'ш'), ('o', 'щ'), ('p', 'з'),
        ('[', 'х'), (']', 'ъ'), ('a', 'ф'), ('s', 'ы'), ('d', 'в'),
        ('f', 'а'), ('g', 'п'), ('h', 'р'), ('j', 'о'), ('k', 'л'),
        ('l', 'д'), (';', 'ж'), ('\'', 'э'), ('z', 'я'), ('x', 'ч'),
        ('c', 'с'), ('v', 'м'), ('b', 'и'), ('n', 'т'), ('m', 'ь'),
        (',', 'б'), ('.', 'ю'), ('/', '.'), ('`', 'ё'),
        // uppercase
        ('Q', 'Й'), ('W', 'Ц'), ('E', 'У'), ('R', 'К'), ('T', 'Е'),
        ('Y', 'Н'), ('U', 'Г'), ('I', 'Ш'), ('O', 'Щ'), ('P', 'З'),
        ('{', 'Х'), ('}', 'Ъ'), ('A', 'Ф'), ('S', 'Ы'), ('D', 'В'),
        ('F', 'А'), ('G', 'П'), ('H', 'Р'), ('J', 'О'), ('K', 'Л'),
        ('L', 'Д'), (':', 'Ж'), ('"', 'Э'), ('Z', 'Я'), ('X', 'Ч'),
        ('C', 'С'), ('V', 'М'), ('B', 'И'), ('N', 'Т'), ('M', 'Ь'),
        ('<', 'Б'), ('>', 'Ю'), ('?', ','), ('~', 'Ё'),
        // symbols on the same physical keys
        ('@', '"'), ('#', '№'), ('$', ';'), ('^', ':'), ('&', '?'),
    ];
    pairs.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> ConversionSet {
        ConversionSet::en_ru()
    }

    #[test]
    fn converts_ghbdtn_to_privet() {
        let t = tables();
        let r = convert("ghbdtn", &t, None, None).unwrap();
        assert_eq!(r.converted, "привет");
        assert_eq!(r.from.as_str(), "us");
        assert_eq!(r.to.as_str(), "ru");
    }

    #[test]
    fn converts_rudds_to_hello() {
        let t = tables();
        let r = convert("руддщ", &t, None, None).unwrap();
        assert_eq!(r.converted, "hello");
        assert_eq!(r.from.as_str(), "ru");
        assert_eq!(r.to.as_str(), "us");
    }

    #[test]
    fn respects_explicit_direction() {
        let t = tables();
        let r = convert(
            "ghbdtn",
            &t,
            Some(&LayoutId::new("us")),
            Some(&LayoutId::new("ru")),
        )
        .unwrap();
        assert_eq!(r.converted, "привет");
    }

    #[test]
    fn keeps_digits_and_punctuation() {
        let t = tables();
        let r = convert("h1!llo", &t, None, None).unwrap();
        assert_eq!(r.converted, "р1!ддщ");
    }

    #[test]
    fn converts_uppercase() {
        let t = tables();
        let r = convert("Ghbdtn", &t, None, None).unwrap();
        assert_eq!(r.converted, "Привет");
    }

    #[test]
    fn empty_text_is_error() {
        let t = tables();
        assert_eq!(convert("", &t, None, None).unwrap_err(), DomainError::EmptyText);
    }

    #[test]
    fn no_letters_is_unknown_direction() {
        let t = tables();
        assert_eq!(
            convert("123 !?", &t, None, None).unwrap_err(),
            DomainError::UnknownDirection
        );
    }

    #[test]
    fn unchanged_text_is_no_input() {
        let t = tables();
        // Digits only mapped? "123" has no map → unchanged → NoInputFound via direction fail.
        // Use a letter that maps to itself? None do. Use explicit direction with unmapped chars.
        let r = convert(
            "123",
            &t,
            Some(&LayoutId::new("us")),
            Some(&LayoutId::new("ru")),
        );
        assert_eq!(r.unwrap_err(), DomainError::NoInputFound);
    }

    #[test]
    fn missing_table_is_error() {
        let t = ConversionSet::default();
        // Direction is detected ("hello" is latin) but no table is loaded.
        let err = convert("hello", &t, None, None).unwrap_err();
        assert!(matches!(err, DomainError::MissingTable { .. }));
        let err = convert(
            "hello",
            &t,
            Some(&LayoutId::new("us")),
            Some(&LayoutId::new("ru")),
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::MissingTable { .. }));
    }

    #[test]
    fn detect_direction_latin() {
        assert_eq!(
            detect_direction("hello"),
            Some((LayoutId::new("us"), LayoutId::new("ru")))
        );
    }

    #[test]
    fn detect_direction_cyrillic() {
        assert_eq!(
            detect_direction("привет"),
            Some((LayoutId::new("ru"), LayoutId::new("us")))
        );
    }

    #[test]
    fn detect_direction_mixed_prefers_majority() {
        // 4 latin, 2 cyrillic → us→ru
        let d = detect_direction("ab cd фы").unwrap();
        assert_eq!(d.0.as_str(), "us");
        // 4 cyrillic, 2 latin → ru→us
        let d = detect_direction("аб вг ef").unwrap();
        assert_eq!(d.0.as_str(), "ru");
    }

    #[test]
    fn detect_direction_empty() {
        assert_eq!(detect_direction(""), None);
        assert_eq!(detect_direction("   "), None);
    }

    #[test]
    fn reverse_map_is_bijective_on_letters() {
        let t = ConversionSet::en_ru();
        let fwd = t.find(&LayoutId::new("us"), &LayoutId::new("ru")).unwrap();
        let rev = t.find(&LayoutId::new("ru"), &LayoutId::new("us")).unwrap();
        for (k, v) in &fwd.map {
            assert_eq!(rev.convert_char(*v), Some(*k), "reverse of {k}->{v}");
        }
    }

    #[test]
    fn roundtrip_en_ru_en() {
        let t = tables();
        let a = convert("hello world", &t, None, None).unwrap();
        assert_eq!(a.converted, "руддщ цщкдв");
        let b = convert(&a.converted, &t, None, None).unwrap();
        assert_eq!(b.converted, "hello world");
    }

    #[test]
    fn symbol_map_at_hash() {
        let t = tables();
        let r = convert(
            "@#$",
            &t,
            Some(&LayoutId::new("us")),
            Some(&LayoutId::new("ru")),
        )
        .unwrap();
        assert_eq!(r.converted, "\"№;");
    }

    #[test]
    fn json_map_roundtrip() {
        let built = en_to_ru_map();
        let json = serde_json::json!({
            "name": "en-ru",
            "from": "us",
            "to": "ru",
            "map": built,
        })
        .to_string();
        let file = MapFile::parse(&json).unwrap();
        assert_eq!(file.name, "en-ru");
        assert_eq!(file.from, "us");
        assert_eq!(file.to, "ru");
        let table = file.into_table();
        assert_eq!(table.convert_char('q'), Some('й'));
        assert_eq!(table.map.len(), built.len());
    }

    #[test]
    fn json_map_bad_is_error() {
        assert!(MapFile::parse("not json").is_err());
        assert!(MapFile::parse(r#"{"name":"x"}"#).is_err());
    }

    #[test]
    fn asset_maps_are_inverses() {
        // Keep assets/maps/{en2ru,ru2en}.json in sync with the built-in map.
        let en2ru = MapFile::parse(include_str!("../../../assets/maps/en2ru.json")).unwrap();
        let ru2en = MapFile::parse(include_str!("../../../assets/maps/ru2en.json")).unwrap();
        assert_eq!(en2ru.from, "us");
        assert_eq!(en2ru.to, "ru");
        assert_eq!(ru2en.from, "ru");
        assert_eq!(ru2en.to, "us");

        let built = en_to_ru_map();
        assert_eq!(en2ru.map.len(), built.len());
        for (k, v) in &built {
            assert_eq!(en2ru.map.get(k), Some(v), "en2ru mismatch for {k:?}");
        }

        for (k, v) in &en2ru.map {
            assert_eq!(ru2en.map.get(v), Some(k), "ru2en not inverse of {k:?}->{v:?}");
        }
    }
}
