//! Composition root: select platform module and wire adapters into use-cases.

use std::sync::Arc;

use mistype_adapters::platform::{create_platform, BuildOptions, PlatformServices};
use mistype_adapters::{ensure_config, load_conversion_set, seed_default_maps, TomlConfigStore};
use mistype_app::config::AppConfig;
use mistype_app::ports::{
    ConfigStore, CorrectorFactory, HeartbeatPort, HotkeySource, LlmHistoryStore, SoundPort,
    TypingBufferPort,
};
use mistype_app::CorrectionService;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

/// Composition root handle. Extra fields are kept for settings reload / shutdown
/// wiring and are part of the daemon API.
pub struct AppContext {
    pub service: Arc<CorrectionService>,
    pub config_tx: watch::Sender<AppConfig>,
    pub buffer: Arc<dyn TypingBufferPort>,
    pub hotkeys: Box<dyn HotkeySource>,
    pub store: Arc<dyn ConfigStore>,
    pub sound: Arc<dyn SoundPort>,
    pub history: Arc<dyn LlmHistoryStore>,
    pub heartbeat: Arc<dyn HeartbeatPort>,
    pub corrector_factory: Arc<dyn CorrectorFactory>,
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
    if let Err(e) = seed_default_maps() {
        warn!("seed default maps failed: {e}");
    }
    let tables = load_conversion_set();
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
    } = services;

    let service = Arc::new(CorrectionService::with_focus(
        injector.clone(),
        clipboard,
        selection,
        buffer.clone() as Arc<dyn TypingBufferPort>,
        layouts,
        notifier,
        focus.clone(),
        tables.clone(),
        config_rx.clone(),
        cfg.general.enabled_on_start,
    ));

    // Ports for sound / history / heartbeat / corrector construction.
    let sound: Arc<dyn SoundPort> = Arc::new(mistype_adapters::SystemSound::new());
    let history: Arc<dyn LlmHistoryStore> = Arc::new(mistype_adapters::FileLlmHistory::new());
    let heartbeat: Arc<dyn HeartbeatPort> = Arc::new(mistype_adapters::FileHeartbeat::new());
    let corrector_factory: Arc<dyn CorrectorFactory> =
        Arc::new(mistype_adapters::OpenAiCorrectorFactory::new());

    // External OpenAI-compatible corrector (preferred for LlmFixSelection).
    install_text_corrector(&service, &cfg, &corrector_factory);

    // Status dump so the operator can see at a glance what is active.
    info!("--- Mistype status ---");
    info!(
        "hotkeys: convert='{}' toggle='{}' llm-fix='{}' consume={}",
        cfg.hotkeys.convert_selection,
        cfg.hotkeys.toggle_enabled,
        cfg.hotkeys.llm_fix_selection,
        cfg.hotkeys.hotkey_consume
    );
    if cfg.llm.enabled && !cfg.llm.base_url.trim().is_empty() {
        info!(
            "llm api: configured (base={}, model={:?})",
            cfg.llm.base_url, cfg.llm.model
        );
    } else {
        info!(
            "llm api: not configured (enabled={}, base_url set={})",
            cfg.llm.enabled,
            !cfg.llm.base_url.trim().is_empty()
        );
    }
    info!(
        "typing_buffer={} max_keys={}  layouts.active={:?}",
        cfg.privacy.typing_buffer, cfg.privacy.typing_buffer_max_keys, cfg.layouts.active
    );
    info!(
        "enabled_on_start={}  fullscreen_guard={}  excluded_apps={:?}",
        cfg.general.enabled_on_start, cfg.behavior.disable_in_fullscreen, cfg.behavior.excluded_apps
    );
    info!("-----------------------");

    info!("Mistype wired up");

    Ok(AppContext {
        service,
        config_tx,
        buffer,
        hotkeys,
        store: config,
        sound,
        history,
        heartbeat,
        corrector_factory,
    })
}

/// Build and install the external OpenAI-compatible corrector from `[llm]`.
/// Called at startup and again on config reload so URL/token/prompt changes
/// take effect without a full restart.
pub fn install_text_corrector(
    service: &Arc<CorrectionService>,
    cfg: &AppConfig,
    factory: &Arc<dyn CorrectorFactory>,
) {
    match factory.build_corrector(&cfg.llm) {
        Some(corrector) => {
            info!(
                "llm-fix selection: backend=openai base={} model={:?}",
                cfg.llm.base_url, cfg.llm.model
            );
            service.set_text_corrector(corrector);
        }
        None => {
            info!(
                "llm-fix selection: openai backend off (enabled={} base_url empty={})",
                cfg.llm.enabled,
                cfg.llm.base_url.trim().is_empty()
            );
        }
    }
}

/// Spawn the command dispatcher for tray commands.
pub fn spawn_tray_dispatcher(
    mut cmd_rx: mpsc::Receiver<mistype_ui::tray::TrayCommand>,
    service: Arc<CorrectionService>,
    on_settings: Arc<dyn Fn() + Send + Sync>,
    on_quit: Arc<dyn Fn() + Send + Sync>,
) {
    tokio::spawn(async move {
        use mistype_domain::CorrectionRequest;
        use mistype_ui::tray::TrayCommand;
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                TrayCommand::ToggleEnabled => {
                    service.toggle_enabled().await;
                }
                TrayCommand::ConvertSelection => {
                    let _ = service.handle(CorrectionRequest::ConvertSelection).await;
                }
                TrayCommand::LlmFixSelection => {
                    let _ = service.handle(CorrectionRequest::LlmFixSelection).await;
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
