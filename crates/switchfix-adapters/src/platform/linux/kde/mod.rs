//! KDE Plasma platform: full implementation via evdev/uinput + org.kde.keyboard.

mod layout;

use std::sync::Arc;

use switchfix_app::config::AppConfig;
use switchfix_app::ports::{
    ClipboardPort, ConfigStore, HotkeySource, LayoutSwitcher, Notifier, SelectionCapture,
    TextInjector, TypingBufferPort,
};
use tracing::{info, warn};

use crate::platform::portable::{
    ArboardClipboard, LogNotifier, ManualHotkeySource, SharedTypingBuffer, TomlConfigStore,
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
        let hotkeys: Box<dyn HotkeySource> = if opts.allow_evdev {
            match EvdevHotkeySource::open_with(EvdevOptions {
                bindings: parsed_bindings,
                consume: cfg.hotkeys.hotkey_consume,
                buffer: buffer.clone(),
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

        let config: Arc<dyn ConfigStore> = Arc::new(TomlConfigStore::open_default());
        let notifier: Arc<dyn Notifier> = Arc::new(LogNotifier);

        Ok(PlatformServices {
            injector,
            clipboard: clipboard.clone() as Arc<dyn ClipboardPort>,
            selection: clipboard as Arc<dyn SelectionCapture>,
            buffer: buffer.clone() as Arc<dyn TypingBufferPort>,
            layouts,
            hotkeys,
            config,
            notifier,
        })
    }
}
