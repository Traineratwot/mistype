//! Composition root: select platform module and wire adapters into use-cases.

use std::sync::Arc;

use switchfix_adapters::platform::{create_platform, BuildOptions, PlatformServices};
use switchfix_adapters::{ensure_config, load_conversion_set, seed_default_maps, TomlConfigStore};
use switchfix_app::config::AppConfig;
use switchfix_app::ports::{ConfigStore, HotkeySource, TypingBufferPort};
use switchfix_app::services::SpellService;
use switchfix_app::CorrectionService;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

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
        tables.clone(),
        config_rx.clone(),
        cfg.general.enabled_on_start,
    ));

    let spell_svc = spell.map(|checker| {
        let svc = Arc::new(SpellService::with_focus(
            checker,
            injector,
            suggestions,
            buffer.clone(),
            tables,
            config_rx.clone(),
            focus,
            cfg.general.enabled_on_start,
        ));
        // Hybrid context layer (LanguageTool / LLM) — optional.
        match switchfix_adapters::platform::portable::build_context_corrector(
            &cfg.spell.backend,
            &cfg.spell.lt_url,
            &cfg.spell.llm_cmd,
        ) {
            Some(ctx) => {
                let name = ctx.name();
                let arc: std::sync::Arc<dyn switchfix_app::ports::ContextCorrector> =
                    std::sync::Arc::from(ctx);
                svc.set_context_corrector(arc.clone());
                svc.set_context_status(true, name);
                // Also feed the «LLM-fix selection» hotkey in CorrectionService.
                service.set_context_corrector(arc);
            }
            None => {
                svc.set_context_status(false, "");
            }
        }
        svc
    });
    if spell_svc.is_some() {
        info!("spellcheck wired (mode={})", cfg.spell.mode);
    } else {
        info!("spellcheck unavailable (no dictionaries or disabled)");
    }

    // Status dump so the operator can see at a glance what is active.
    info!("--- SwitchFix status ---");
    info!(
        "hotkeys: convert='{}' toggle='{}' fix='{}' consume={}",
        cfg.hotkeys.convert_selection,
        cfg.hotkeys.toggle_enabled,
        cfg.hotkeys.fix_spelling,
        cfg.hotkeys.hotkey_consume
    );
    info!(
        "spell: enabled={} mode='{}' backend='{}' langs={:?} min_word_len={} suggest_count={} auto_fix_min_len={} sound={}",
        cfg.spell.enabled,
        cfg.spell.mode,
        cfg.spell.backend,
        cfg.spell.languages,
        cfg.spell.min_word_len,
        cfg.spell.suggest_count,
        cfg.spell.auto_fix_min_len,
        cfg.spell.sound
    );
    if !cfg.spell.lt_url.is_empty() {
        info!("spell context: languagetool url={}", cfg.spell.lt_url);
    }
    if !cfg.spell.llm_cmd.is_empty() {
        info!("spell context: llm cmd={}", cfg.spell.llm_cmd);
    }
    info!(
        "typing_buffer={} max_keys={}  layouts.active={:?}",
        cfg.privacy.typing_buffer, cfg.privacy.typing_buffer_max_keys, cfg.layouts.active
    );
    info!(
        "enabled_on_start={}  fullscreen_guard={}  excluded_apps={:?}",
        cfg.general.enabled_on_start, cfg.behavior.disable_in_fullscreen, cfg.behavior.excluded_apps
    );
    if spell_svc.is_some() {
        info!("suggestions UI: plasmoid / tray menu / Ctrl+1..N; peek via `switchfix suggest-peek`");
    }
    info!("-----------------------");

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
    spell: Option<Arc<SpellService>>,
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
                    if let Some(s) = &spell {
                        s.set_enabled(service.is_enabled());
                    }
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
                TrayCommand::AcceptSuggestion(n) => {
                    if let Some(s) = &spell {
                        let action = s.accept_slot(n).await;
                        info!("tray accept {n}: {action:?}");
                    }
                }
                TrayCommand::LearnWord => {
                    if let Some(s) = &spell {
                        let action = s.add_to_dictionary().await;
                        info!("tray learn: {action:?}");
                    }
                }
                TrayCommand::DismissSuggestions => {
                    if let Some(s) = &spell {
                        s.dismiss().await;
                    }
                }
            }
        }
    });
}
