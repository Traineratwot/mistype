//! Composition root: select platform module and wire adapters into use-cases.

use std::sync::Arc;

use switchfix_adapters::platform::{create_platform, BuildOptions, PlatformServices};
use switchfix_adapters::{ensure_config, TomlConfigStore};
use switchfix_app::config::AppConfig;
use switchfix_app::ports::{ConfigStore, HotkeySource, TypingBufferPort};
use switchfix_app::services::SpellService;
use switchfix_app::CorrectionService;
use switchfix_domain::ConversionSet;
use tokio::sync::{mpsc, watch};
use tracing::info;

/// Composition root handle. Extra fields are kept for settings reload / shutdown
/// wiring and are part of the daemon API.
#[allow(dead_code)]
pub struct AppContext {
    pub service: Arc<CorrectionService>,
    pub spell: Option<Arc<SpellService>>,
    pub config_tx: watch::Sender<AppConfig>,
    pub config_rx: watch::Receiver<AppConfig>,
    pub buffer: Arc<dyn TypingBufferPort>,
    pub hotkeys: Box<dyn HotkeySource>,
    pub store: Arc<dyn ConfigStore>,
    pub shutdown: tokio::sync::Notify,
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

    let platform = create_platform();
    let info = platform.info();
    info!(
        "platform: {} (supported={})",
        info.id, info.supported
    );

    let services = platform
        .build_services(
            &cfg,
            &BuildOptions {
                allow_evdev: opts.allow_evdev,
                allow_uinput: opts.allow_uinput,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let PlatformServices {
        injector,
        clipboard,
        selection,
        buffer,
        layouts,
        hotkeys,
        config,
        notifier,
        focus,
        spell,
        suggestions,
    } = services;

    let service = Arc::new(CorrectionService::with_focus(
        injector.clone(),
        clipboard,
        selection,
        buffer.clone() as Arc<dyn TypingBufferPort>,
        layouts,
        notifier,
        focus.clone(),
        ConversionSet::en_ru(),
        config_rx.clone(),
        cfg.general.enabled_on_start,
    ));

    let spell_svc = spell.map(|checker| {
        Arc::new(SpellService::with_focus(
            checker,
            injector,
            suggestions,
            buffer.clone(),
            ConversionSet::en_ru(),
            config_rx.clone(),
            focus,
            cfg.general.enabled_on_start,
        ))
    });
    if spell_svc.is_some() {
        info!("spellcheck wired (mode={})", cfg.spell.mode);
    } else {
        info!("spellcheck unavailable (no dictionaries or disabled)");
    }

    info!("SwitchFix wired up");

    Ok(AppContext {
        service,
        spell: spell_svc,
        config_tx,
        config_rx,
        buffer,
        hotkeys,
        store: config,
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
                TrayCommand::OpenSettings => on_settings(),
                TrayCommand::Quit => {
                    on_quit();
                    break;
                }
            }
        }
    });
}
