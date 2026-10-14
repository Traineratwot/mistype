//! TOML config store at `~/.config/switchfix/config.toml`.

use std::path::PathBuf;

use switchfix_app::config::AppConfig;
use switchfix_app::ports::{ConfigError, ConfigStore};
use tracing::{debug, warn};

pub struct TomlConfigStore {
    path: PathBuf,
}

impl TomlConfigStore {
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("switchfix")
            .join("config.toml")
    }

    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn open_default() -> Self {
        Self::new(Self::default_path())
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl ConfigStore for TomlConfigStore {
    fn load(&self) -> Result<AppConfig, ConfigError> {
        if !self.path.exists() {
            debug!("config missing at {:?}, using defaults", self.path);
            return Ok(AppConfig::default());
        }
        let raw = std::fs::read_to_string(&self.path)
            .map_err(|e| ConfigError::Io(e.to_string()))?;
        toml::from_str(&raw).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    fn save(&self, cfg: &AppConfig) -> Result<(), ConfigError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::Io(e.to_string()))?;
        }
        let raw = toml::to_string_pretty(cfg).map_err(|e| ConfigError::Parse(e.to_string()))?;
        std::fs::write(&self.path, raw).map_err(|e| ConfigError::Io(e.to_string()))?;
        debug!("config saved to {:?}", self.path);
        Ok(())
    }
}

/// Ensure a config file exists (write defaults if missing).
pub fn ensure_config(store: &TomlConfigStore) -> AppConfig {
    match store.load() {
        Ok(c) => c,
        Err(e) => {
            warn!("config load failed ({e}), writing defaults");
            let cfg = AppConfig::default();
            let _ = store.save(&cfg);
            cfg
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = TomlConfigStore::new(dir.path().join("config.toml"));
        let cfg = AppConfig::default();
        store.save(&cfg).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = TomlConfigStore::new(dir.path().join("nope.toml"));
        let cfg = store.load().unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn parses_partial_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[general]\nenabled_on_start = false\nlanguage = \"en\"\n",
        )
        .unwrap();
        let store = TomlConfigStore::new(path);
        let cfg = store.load().unwrap();
        assert!(!cfg.general.enabled_on_start);
        assert_eq!(cfg.general.language, "en");
        // defaults filled in
        assert_eq!(cfg.hotkeys.convert_selection, "ctrl+f1");
    }
}
