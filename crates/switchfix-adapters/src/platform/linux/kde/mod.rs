//! KDE Plasma platform: full implementation via evdev/uinput + org.kde.keyboard.

mod focus;
mod layout;

use std::sync::Arc;

use switchfix_app::config::AppConfig;
use switchfix_app::ports::{
    ClipboardPort, ConfigStore, FocusMonitor, HotkeySource, LayoutSwitcher, Notifier,
    SelectionCapture, TextInjector, TypingBufferPort,
};
use tracing::{debug, info, warn};

use crate::platform::portable::{
    ArboardClipboard, LayoutCharMapper, LogNotifier, ManualHotkeySource, SharedSuggestionPresenter,
    SharedTypingBuffer, SuggestionState, TomlConfigStore, ZspellDictionary,
};
use crate::platform::{BuildOptions, Platform, PlatformError, PlatformInfo, PlatformServices};

use super::evdev_hotkey::{EvdevHotkeySource, EvdevOptions};
use super::uinput_injector::UinputInjector;
use layout::KdeLayoutSwitcher;

pub struct KdePlatform {
    info: PlatformInfo,
}

impl KdePlatform {
    pub fn new(info: PlatformInfo) -> Self {
        Self { info }
    }
}

#[async_trait::async_trait]
impl Platform for KdePlatform {
    fn info(&self) -> PlatformInfo {
        self.info.clone()
    }

    async fn build_services(
        &self,
        cfg: &AppConfig,
        opts: &BuildOptions,
    ) -> Result<PlatformServices, PlatformError> {
        // Typing buffer
        let buffer = SharedTypingBuffer::new(
            cfg.privacy.typing_buffer_max_keys,
            cfg.privacy.typing_buffer,
        );

        // Layouts (KDE D-Bus)
        let layouts: Arc<dyn LayoutSwitcher> = match KdeLayoutSwitcher::connect().await {
            Ok(l) => {
                let list = l.layouts().await.unwrap_or_default();
                info!(
                    "KDE layouts connected: {:?}",
                    list.iter()
                        .map(|i| i.id.as_str().to_string())
                        .collect::<Vec<_>>()
                );
                Arc::new(l)
            }
            Err(e) => {
                warn!("KDE layout switcher unavailable: {e}");
                return Err(PlatformError::Layout(format!(
                    "org.kde.keyboard D-Bus unavailable: {e}. Are you on KDE Plasma?"
                )));
            }
        };

        // Layout-aware char mapper: keycode → actual character of the active layout.
        let layout_ids: Vec<String> = layouts
            .layouts()
            .await
            .unwrap_or_default()
            .iter()
            .map(|i| i.id.as_str().to_string())
            .collect();
        let current_idx = layouts.current().await.map(|i| i.as_u32()).unwrap_or(0);
        let mapper = Arc::new(LayoutCharMapper::new(layout_ids, current_idx));
        {
            let mapper = mapper.clone();
            let mut rx = layouts.subscribe();
            tokio::spawn(async move {
                while let Ok(idx) = rx.recv().await {
                    debug!("layout mapper -> index {}", idx.as_u32());
                    mapper.set_current(idx.as_u32());
                }
            });
        }

        // Injector
        let injector: Arc<dyn TextInjector> = if opts.allow_uinput {
            match UinputInjector::open() {
                Ok(i) => {
                    info!("uinput virtual keyboard ready");
                    Arc::new(i)
                }
                Err(e) => {
                    warn!("uinput unavailable: {e}");
                    return Err(PlatformError::Injection(format!(
                        "uinput unavailable: {e}. Run: sudo usermod -aG input $USER"
                    )));
                }
            }
        } else {
            return Err(PlatformError::Injection("uinput disabled by options".into()));
        };

        // Second uinput for clipboard selection-capture chords (uinput allows multiple).
        let injector_for_clip: Box<dyn TextInjector> = match UinputInjector::open() {
            Ok(i) => Box::new(i),
            Err(e) => {
                warn!("second uinput for clipboard failed: {e}");
                return Err(PlatformError::Clipboard(format!(
                    "uinput for clipboard: {e}"
                )));
            }
        };
        let clipboard = Arc::new(ArboardClipboard::new(
            Some(injector_for_clip),
            cfg.behavior.copy_settle_ms,
        ));

        // Hotkeys
        let parsed_bindings = super::parse_hotkey_bindings(cfg);
        let dynamic_bindings = super::dynamic_suggestion_bindings(cfg.spell.suggest_count);
        let suggestion_state = SuggestionState::new();
        let presenter = SharedSuggestionPresenter::new(suggestion_state);
        let bar_visible = presenter.visible_flag();
        let hotkeys: Box<dyn HotkeySource> = if opts.allow_evdev {
            match EvdevHotkeySource::open_with(EvdevOptions {
                bindings: parsed_bindings,
                dynamic_bindings,
                bar_visible,
                consume: cfg.hotkeys.hotkey_consume,
                buffer: buffer.clone(),
                mapper: mapper.clone(),
            }) {
                Ok(h) => {
                    info!(
                        "evdev hotkeys active (consume={})",
                        cfg.hotkeys.hotkey_consume
                    );
                    Box::new(h)
                }
                Err(e) => {
                    tracing::error!(
                        "evdev unavailable ({e}); hotkeys will NOT work from the keyboard. \
                         Fix: log out/in so the session has group 'input', or run: sg input -c switchfix daemon"
                    );
                    Box::new(ManualHotkeySource::new())
                }
            }
        } else {
            Box::new(ManualHotkeySource::new())
        };

        // Spell dictionaries (optional — degrade gracefully).
        let spell = if cfg.spell.enabled {
            let user_dic = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("switchfix")
                .join("user.dic");
            let d = ZspellDictionary::load(&cfg.spell.languages, user_dic);
            if d.has_any() {
                Some(d as Arc<dyn switchfix_app::ports::SpellChecker>)
            } else {
                warn!("no spell dictionaries found — spellcheck disabled");
                None
            }
        } else {
            None
        };

        let config: Arc<dyn ConfigStore> = Arc::new(TomlConfigStore::open_default());
        let notifier: Arc<dyn Notifier> = Arc::new(LogNotifier);

        let focus: Option<Arc<dyn FocusMonitor>> =
            match focus::KdeFocusMonitor::try_connect().await {
                Some(f) => {
                    info!("focus monitor ready (KWin/xprop)");
                    Some(Arc::new(f))
                }
                None => {
                    warn!("focus monitor unavailable — no app filtering");
                    None
                }
            };

        Ok(PlatformServices {
            injector,
            clipboard: clipboard.clone() as Arc<dyn ClipboardPort>,
            selection: clipboard as Arc<dyn SelectionCapture>,
            buffer: buffer.clone() as Arc<dyn TypingBufferPort>,
            layouts,
            hotkeys,
            config,
            notifier,
            focus,
            spell,
            suggestions: Arc::new(presenter) as Arc<dyn switchfix_app::ports::SuggestionPresenter>,
        })
    }
}
