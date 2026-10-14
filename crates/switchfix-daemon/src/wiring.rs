//! Composition root: wire adapters into use-cases.

use std::sync::Arc;

use switchfix_adapters::config_store::{ensure_config, TomlConfigStore};
use switchfix_adapters::evdev_hotkey::{EvdevHotkeySource, SharedTypingBuffer};
use switchfix_adapters::kde_layout::KdeLayoutSwitcher;
use switchfix_adapters::uinput_injector::UinputInjector;
use switchfix_adapters::WaylandClipboard;
use switchfix_app::config::AppConfig;
use switchfix_app::ports::{
    ClipboardPort, HotkeySource, LayoutSwitcher, NotifyError, NotifyLevel, Notifier, TextInjector,
    TypingBufferPort,
};
use switchfix_app::CorrectionService;
use switchfix_domain::ConversionSet;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

pub struct AppContext {
    pub service: Arc<CorrectionService>,
    pub config_tx: watch::Sender<AppConfig>,
    pub config_rx: watch::Receiver<AppConfig>,
    pub buffer: Arc<SharedTypingBuffer>,
    pub hotkeys: Box<dyn HotkeySource>,
    pub store: Arc<TomlConfigStore>,
    pub shutdown: tokio::sync::Notify,
}

/// Log-based notifier (desktop notifications can be added later).
struct LogNotifier;

#[async_trait::async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, msg: &str, level: NotifyLevel) -> Result<(), NotifyError> {
        match level {
            NotifyLevel::Info => info!(target: "switchfix", "{msg}"),
            NotifyLevel::Warning => warn!(target: "switchfix", "{msg}"),
            NotifyLevel::Error => tracing::error!(target: "switchfix", "{msg}"),
        }
        Ok(())
    }
}

pub struct WireOptions {
    pub allow_evdev: bool,
    pub allow_uinput: bool,
}

impl Default for WireOptions {
    fn default() -> Self {
        Self {
            allow_evdev: true,
            allow_uinput: true,
        }
    }
}

pub async fn build(opts: WireOptions) -> anyhow::Result<AppContext> {
    let store = Arc::new(TomlConfigStore::open_default());
    let cfg = ensure_config(&store);
    let (config_tx, config_rx) = watch::channel(cfg.clone());

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
                list.iter().map(|i| i.id.as_str().to_string()).collect::<Vec<_>>()
            );
            Arc::new(l)
        }
        Err(e) => {
            warn!("KDE layout switcher unavailable: {e}");
            return Err(anyhow::anyhow!(
                "org.kde.keyboard D-Bus unavailable: {e}. Are you on KDE Plasma?"
            ));
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
                return Err(anyhow::anyhow!(
                    "uinput unavailable: {e}. Run: sudo usermod -aG input $USER"
                ));
            }
        }
    } else {
        return Err(anyhow::anyhow!("uinput disabled by options"));
    };

    // Clipboard + selection capture (shares injector)
    // We need a separate Arc for the injector inside clipboard — clone via new UinputInjector?
    // Instead wrap: create clipboard that uses the same injector reference.
    // Simpler: create WaylandClipboard with a second injector open (uinput allows multiple).
    let injector_for_clip: Box<dyn TextInjector> = match UinputInjector::open() {
        Ok(i) => Box::new(i),
        Err(e) => {
            warn!("second uinput for clipboard failed: {e}");
            return Err(anyhow::anyhow!("uinput for clipboard: {e}"));
        }
    };
    let clipboard = Arc::new(WaylandClipboard::new(
        Some(injector_for_clip),
        cfg.behavior.copy_settle_ms,
    ));

    // Hotkeys
    let hotkeys: Box<dyn HotkeySource> = if opts.allow_evdev {
        match EvdevHotkeySource::open(buffer.clone()) {
            Ok(h) => {
                info!("evdev hotkeys active");
                Box::new(h)
            }
            Err(e) => {
                warn!("evdev unavailable ({e}); hotkeys will only work via tray/CLI");
                Box::new(switchfix_adapters::evdev_hotkey::ManualHotkeySource::new())
            }
        }
    } else {
        Box::new(switchfix_adapters::evdev_hotkey::ManualHotkeySource::new())
    };

    let notifier = Arc::new(LogNotifier);

    let service = Arc::new(CorrectionService::new(
        injector,
        clipboard.clone() as Arc<dyn ClipboardPort>,
        clipboard as Arc<dyn switchfix_app::ports::SelectionCapture>,
        buffer.clone() as Arc<dyn TypingBufferPort>,
        layouts,
        notifier,
        ConversionSet::en_ru(),
        config_rx.clone(),
        cfg.general.enabled_on_start,
    ));

    info!("SwitchFix wired up");

    Ok(AppContext {
        service,
        config_tx,
        config_rx,
        buffer,
        hotkeys,
        store,
        shutdown: tokio::sync::Notify::new(),
    })
}

/// Spawn the command dispatcher for tray commands.
pub fn spawn_tray_dispatcher(
    mut cmd_rx: mpsc::Receiver<switchfix_ui::tray::TrayCommand>,
    service: Arc<CorrectionService>,
    on_settings: Arc<dyn Fn() + Send + Sync>,
    on_quit: Arc<dyn Fn() + Send + Sync>,
) {
    tokio::spawn(async move {
        use switchfix_domain::CorrectionRequest;
        use switchfix_ui::tray::TrayCommand;
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                TrayCommand::ToggleEnabled => {
                    service.toggle_enabled().await;
                }
                TrayCommand::ConvertSelection => {
                    let _ = service.handle(CorrectionRequest::ConvertSelection).await;
                }
                TrayCommand::ConvertLastWord => {
                    let _ = service.handle(CorrectionRequest::ConvertLastWord).await;
                }
                TrayCommand::OpenSettings => on_settings(),
                TrayCommand::Quit => {
                    on_quit();
                    break;
                }
            }
        }
    });
}
