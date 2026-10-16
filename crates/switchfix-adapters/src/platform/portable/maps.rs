//! User-editable layout maps under `~/.config/switchfix/maps/`.
//!
//! Defaults are seeded once when the directory is empty. Subsequent edits are
//! never overwritten; deleted files are not restored.

use std::path::PathBuf;

use switchfix_domain::{en_to_ru_map, ConversionSet, MapFile};
use tracing::{info, warn};

const EN2RU: &str = "en2ru.json";
const RU2EN: &str = "ru2en.json";

pub fn user_maps_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("switchfix")
        .join("maps")
}

fn default_en_ru_file() -> MapFile {
    MapFile {
        name: "en-ru".into(),
        from: "us".into(),
        to: "ru".into(),
        map: en_to_ru_map(),
    }
}

fn default_ru_en_file() -> MapFile {
    let en = default_en_ru_file();
    MapFile {
        name: "ru-en".into(),
        from: en.to,
        to: en.from,
        map: en.map.into_iter().map(|(k, v)| (v, k)).collect(),
    }
}

/// Write default maps into `dir` only when it has no `*.json` yet.
/// Never overwrites user edits or restores intentionally deleted files.
pub fn seed_default_maps_in(dir: &std::path::Path) -> std::io::Result<()> {
    let has_json = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| {
                e.path()
                    .extension()
                    .map(|x| x == "json")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    if has_json {
        return Ok(());
    }

    std::fs::create_dir_all(dir)?;
    for file in [default_en_ru_file(), default_ru_en_file()] {
        let name = if file.from == "us" { EN2RU } else { RU2EN };
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        let raw = serde_json::to_string_pretty(&file)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&path, raw)?;
        info!("seeded default map {:?}", path);
    }
    Ok(())
}

/// Seed the standard user maps directory.
pub fn seed_default_maps() -> std::io::Result<()> {
    seed_default_maps_in(&user_maps_dir())
}

/// Load conversion tables from `*.json` in `dir`.
/// Invalid files are skipped with a warning; if nothing loads, fall back to
/// the built-in EN↔RU table so the app stays usable.
pub fn load_conversion_set_from(dir: &std::path::Path) -> ConversionSet {
    let mut tables = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!("maps dir {:?} unreadable: {e}", dir);
            return ConversionSet::en_ru();
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    paths.sort();

    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(raw) => match MapFile::parse(&raw) {
                Ok(file) => {
                    if file.map.is_empty() {
                        warn!("map {:?} has empty map, skipped", path);
                        continue;
                    }
                    tables.push(file.into_table());
                }
                Err(e) => warn!("map {:?} invalid: {e}", path),
            },
            Err(e) => warn!("map {:?} unreadable: {e}", path),
        }
    }

    if tables.is_empty() {
        warn!("no valid maps in {:?}, using built-in en↔ru", dir);
        return ConversionSet::en_ru();
    }

    // Ensure both directions when only one file is present.
    let mut all = tables.clone();
    for t in &tables {
        let rev = t.reverse();
        if !all.iter().any(|x| x.from == rev.from && x.to == rev.to) {
            all.push(rev);
        }
    }
    ConversionSet::new(all)
}

/// Load maps from the standard user directory.
pub fn load_conversion_set() -> ConversionSet {
    load_conversion_set_from(&user_maps_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_creates_two_files() {
        let dir = tempfile::tempdir().unwrap();
        let maps = dir.path().join("maps");
        seed_default_maps_in(&maps).unwrap();
        assert!(maps.join(EN2RU).exists());
        assert!(maps.join(RU2EN).exists());
    }

    #[test]
    fn second_seed_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let maps = dir.path().join("maps");
        seed_default_maps_in(&maps).unwrap();
        let edited = r#"{"name":"en-ru","from":"us","to":"ru","map":{"q":"X"}}"#;
        std::fs::write(maps.join(EN2RU), edited).unwrap();
        seed_default_maps_in(&maps).unwrap();
        let raw = std::fs::read_to_string(maps.join(EN2RU)).unwrap();
        assert!(raw.contains("\"X\""), "seed must not clobber user edit");
    }

    #[test]
    fn load_after_seed_matches_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let maps = dir.path().join("maps");
        seed_default_maps_in(&maps).unwrap();
        let set = load_conversion_set_from(&maps);
        let r = switchfix_domain::convert("ghbdtn", &set, None, None).unwrap();
        assert_eq!(r.converted, "привет");
    }

    #[test]
    fn edited_map_is_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let maps = dir.path().join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::write(
            maps.join(EN2RU),
            r#"{"name":"en-ru","from":"us","to":"ru","map":{"q":"X"}}"#,
        )
        .unwrap();
        let set = load_conversion_set_from(&maps);
        let r = switchfix_domain::convert("q", &set, None, None).unwrap();
        assert_eq!(r.converted, "X");
        // reverse direction (ru→us) is synthesized from the forward map
        let from = switchfix_domain::LayoutId::new("ru");
        let to = switchfix_domain::LayoutId::new("us");
        let r = switchfix_domain::convert("X", &set, Some(&from), Some(&to)).unwrap();
        assert_eq!(r.converted, "q");
    }

    #[test]
    fn bad_json_skipped_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let maps = dir.path().join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        std::fs::write(maps.join("en2ru.json"), "not json").unwrap();
        let set = load_conversion_set_from(&maps);
        let r = switchfix_domain::convert("ghbdtn", &set, None, None).unwrap();
        assert_eq!(r.converted, "привет");
    }

    #[test]
    fn missing_dir_falls_back() {
        let dir = std::path::Path::new("/no/such/switchfix/maps");
        let set = load_conversion_set_from(dir);
        assert!(!set.is_empty());
    }
}
