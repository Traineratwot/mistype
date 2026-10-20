//! SwitchFix — fix text typed in the wrong keyboard layout.

mod doctor;
mod setup;
mod wiring;

use clap::{Parser, Subcommand};
use switchfix_domain::CorrectionRequest;
use switchfix_ui::tray::TrayModel;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "switchfix", version, about = "Fix text typed in the wrong keyboard layout")]
struct Cli {
    /// Verbose logging (debug) — shows hotkey and LLM decisions at runtime.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the tray daemon (default).
    Daemon,
    /// Open the settings window.
    Settings,
    /// Convert text from stdin or --text and print the result (no injection).
    Convert {
        /// Text to convert. If omitted, reads stdin.
        #[arg(long)]
        text: Option<String>,
    },
    /// Print diagnostics.
    Doctor,
    /// Interactive setup (input group, udev rule, user service). Asks for sudo only when needed.
    Setup {
        /// Answer "yes" to all prompts (sudo password may still be requested).
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Toggle enabled/paused state of a running daemon (via file flag for now).
    Toggle,
    /// Ask a running daemon to reload config.toml and layout maps.
    Reload,
    /// Play the hotkey feedback sound (and print a note) — smoke-test audio.
    Beep,
    /// Inject a synthetic hotkey (ctrl+f20) and verify evdev interception works.
    Selftest,
    /// Print LLM-fix history as JSON (for the Plasma widget). Newest first.
    LlmHistory,
    /// Copy a field from one history entry to the clipboard (for the widget).
    /// Index is 0 = newest. Field: `from` (original) or `to` (result).
    LlmCopy {
        /// Entry index, 0 = newest.
        #[arg(long, default_value_t = 0)]
        index: usize,
        /// Which text: `from` or `to`.
        #[arg(long, default_value = "to")]
        field: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let default_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(default_level)),
        )
        .init();
    if cli.verbose {
        info!("verbose logging on (RUST_LOG overrides if set)");
    }

    match cli.command.unwrap_or(Command::Daemon) {
        Command::Doctor => {
            let report = doctor::run().await;
            report.print();
            if !report.all_ok() {
                std::process::exit(1);
            }
        }
        Command::Setup { yes } => {
            let report = setup::run(yes);
            report.print();
        }
        Command::Convert { text } => {
            let input = match text {
                Some(t) => t,
                None => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s.trim_end_matches('\n').to_string()
                }
            };
            let _ = switchfix_adapters::seed_default_maps();
            let tables = switchfix_adapters::load_conversion_set();
            match switchfix_domain::convert(&input, &tables, None, None) {
                Ok(c) => {
                    println!("{}", c.converted);
                    eprintln!(
                        "# direction: {} -> {}",
                        c.from.as_str(),
                        c.to.as_str()
                    );
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            }
        }
        Command::Settings => {
            let store = switchfix_adapters::TomlConfigStore::open_default();
            let cfg = switchfix_adapters::ensure_config(&store);
            let _ = switchfix_adapters::seed_default_maps();
            info!("opening settings (config at {:?})", store.path());
            let config_dir = store
                .path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| {
                    dirs::config_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join("switchfix")
                });
            let probe = std::sync::Arc::new(
                switchfix_adapters::platform::portable::OpenAiCorrector::new(&cfg.llm),
            );
            switchfix_ui::run_settings(Box::new(store), config_dir, Some(probe))?;
        }
        Command::Reload => {
            let path = std::env::temp_dir().join("switchfix.reload");
            let token = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            std::fs::write(&path, token.to_string())?;
            println!("reload signaled");
        }
        Command::Toggle => {
            // Simple toggle via a state file the daemon watches (MVP).
            let path = std::env::temp_dir().join("switchfix.enabled");
            let current = std::fs::read_to_string(&path)
                .map(|s| s.trim() != "0")
                .unwrap_or(true);
            let next = !current;
            std::fs::write(&path, if next { "1" } else { "0" })?;
            println!("{}", if next { "enabled" } else { "paused" });
        }
        Command::Beep => {
            println!("playing hotkey feedback sound…");
            switchfix_ui::play_hotkey_sound();
            // Give the player a moment so the process isn't killed on exit.
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            println!("done (if you heard nothing, install canberra-gtk-play or paplay)");
        }
        Command::Selftest => {
            println!("SwitchFix hotkey self-test");
            println!("=========================");
            let report = switchfix_adapters::platform::linux::selftest::run_hotkey_selftest().await;
            report.print();
            if !report.all_ok() {
                eprintln!("\nself-test FAILED");
                std::process::exit(1);
            }
            println!("\nself-test OK — hotkey interception works");
        }
        Command::LlmHistory => {
            let list = switchfix_adapters::read_llm_history();
            // Compact JSON for the Plasma widget.
            println!("{}", serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()));
        }
        Command::LlmCopy { index, field } => {
            let list = switchfix_adapters::read_llm_history();
            let Some(entry) = list.get(index) else {
                eprintln!("no history entry at index {index}");
                std::process::exit(1);
            };
            let text = match field.as_str() {
                "from" => entry.from.as_str(),
                _ => entry.to.as_str(),
            };
            // Use the same clipboard path as the daemon (arboard).
            let mut clip = arboard::Clipboard::new().map_err(|e| anyhow::anyhow!("clipboard: {e}"))?;
            clip.set_text(text.to_string())
                .map_err(|e| anyhow::anyhow!("clipboard set: {e}"))?;
            println!("copied ({} chars)", text.chars().count());
        }
        Command::Daemon => {
            run_daemon().await?;
        }
    }

    Ok(())
}

/// Persist an LLM-fix outcome for the Plasma widget history panel.
fn record_llm_history(out: &switchfix_domain::CorrectionOutcome) {
    use switchfix_domain::CorrectionOutcome;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    match out {
        CorrectionOutcome::Converted {
            original,
            converted,
            ..
        } => switchfix_adapters::append_llm_history(switchfix_adapters::LlmHistoryEntry {
            ts,
            from: original.clone(),
            to: converted.clone(),
            ok: true,
        }),
        CorrectionOutcome::Partial {
            original,
            converted,
            ..
        } => switchfix_adapters::append_llm_history(switchfix_adapters::LlmHistoryEntry {
            ts,
            from: original.clone(),
            to: converted.clone(),
            ok: true,
        }),
        CorrectionOutcome::Failed(e) => switchfix_adapters::append_llm_history(
            switchfix_adapters::LlmHistoryEntry {
                ts,
                from: String::new(),
                to: format!("error: {e}"),
                ok: false,
            },
        ),
        CorrectionOutcome::NoInputFound | CorrectionOutcome::Disabled => {}
    }
}

async fn run_daemon() -> anyhow::Result<()> {
    info!("starting SwitchFix daemon");

    let ctx = wiring::build(wiring::WireOptions::default()).await?;

    // Tray
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(32);
    let tray_model = TrayModel::new(cmd_tx, ctx.service.is_enabled());
    let tray_handle = switchfix_ui::tray::spawn_tray(tray_model.clone())
        .await
        .map_err(|e| anyhow::anyhow!("tray: {e}"))?;

    // Quit coordination
    let quit = std::sync::Arc::new(tokio::sync::Notify::new());
    let quit2 = quit.clone();
    let on_settings = std::sync::Arc::new(|| {
        let _ = std::process::Command::new(
            std::env::current_exe().unwrap_or_else(|_| "switchfix".into()),
        )
        .arg("settings")
        .spawn();
    });
    let on_quit = std::sync::Arc::new(move || {
        quit2.notify_one();
    });

    wiring::spawn_tray_dispatcher(
        cmd_rx,
        ctx.service.clone(),
        on_settings,
        on_quit,
    );

    // Hotkey loop
    let mut events = ctx.hotkeys.events();
    let service = ctx.service.clone();
    let buffer = ctx.buffer.clone();

    // Word events → LLM undo (Backspace) + cancel-on-typing.
    {
        let mut word_rx = buffer.word_events();
        let svc = service.clone();
        tokio::spawn(async move {
            use switchfix_app::ports::WordEvent;
            while let Ok(ev) = word_rx.recv().await {
                match ev {
                    WordEvent::Backspace => {
                        if svc.on_backspace().await {
                            switchfix_ui::play_fix_sound();
                        }
                    }
                    WordEvent::Updated(_) => svc.on_typing(),
                    WordEvent::Completed { .. } => {}
                }
            }
        });
    }

    // Also poll the toggle state file for `switchfix toggle`.
    let toggle_path = std::env::temp_dir().join("switchfix.enabled");
    let mut last_toggle = std::fs::read_to_string(&toggle_path).unwrap_or_else(|_| "1".into());
    let reload_path = std::env::temp_dir().join("switchfix.reload");
    let mut last_reload = std::fs::read_to_string(&reload_path).unwrap_or_default();
    // Settings "Сохранить" signals a full restart so hotkeys rebind (re-exec).
    let restart_path = std::env::temp_dir().join("switchfix.restart");
    let last_restart = std::fs::read_to_string(&restart_path).unwrap_or_default();
    switchfix_adapters::write_daemon_heartbeat(service.is_enabled());

    let mut restart_requested = false;
    loop {
        tokio::select! {
            _ = quit.notified() => {
                info!("quit requested");
                break;
            }
            ev = events.recv() => {
                match ev {
                    Some(he) => {
                        use switchfix_app::ports::HotkeyEvent;
                        info!("hotkey received: {he:?}");
                        // Immediate feedback: blink tray + short sound.
                        tray_handle.flash(switchfix_ui::FLASH_DURATION);
                        switchfix_ui::play_hotkey_sound();

                        match he {
                            HotkeyEvent::ConvertSelection => {
                                let out = service.handle(CorrectionRequest::ConvertSelection).await;
                                info!("correction outcome: {out:?}");
                            }
                            HotkeyEvent::LlmFixSelection => {
                                // Yellow blink while the LLM thinks, then replace.
                                tray_handle.set_busy(true);
                                let out = service
                                    .handle(CorrectionRequest::LlmFixSelection)
                                    .await;
                                tray_handle.set_busy(false);
                                info!("llm-fix outcome: {out:?}");
                                record_llm_history(&out);
                                if out.is_success() {
                                    switchfix_ui::play_fix_sound();
                                }
                            }
                            HotkeyEvent::ToggleEnabled => {
                                service.toggle_enabled().await;
                                switchfix_adapters::write_daemon_heartbeat(service.is_enabled());
                            }
                        }
                    }
                    None => {
                        warn!("hotkey channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                switchfix_adapters::write_daemon_heartbeat(service.is_enabled());
                if let Ok(v) = std::fs::read_to_string(&toggle_path) {
                    if v != last_toggle {
                        last_toggle = v;
                        service.toggle_enabled().await;
                        switchfix_adapters::write_daemon_heartbeat(service.is_enabled());
                    }
                }
                // Config / maps reload (settings button or `switchfix reload`).
                if let Ok(token) = std::fs::read_to_string(&reload_path) {
                    if token != last_reload {
                        last_reload = token.clone();
                        match ctx.store.load() {
                            Ok(cfg) => {
                                // Rebuild the OpenAI corrector so URL/token/prompt
                                // changes apply without a full daemon restart.
                                crate::wiring::install_text_corrector(&service, &cfg);
                                ctx.config_tx.send_replace(cfg);
                                info!("config reloaded from disk");
                            }
                            Err(e) => warn!("config reload failed, keeping previous: {e}"),
                        }
                        let tables = switchfix_adapters::load_conversion_set();
                        service.replace_tables(tables);
                        info!("layout maps reloaded (hotkeys need Settings «Сохранить» or a daemon restart)");
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {
                // Settings Save: full restart so hotkeys / consume rebind.
                if let Ok(token) = std::fs::read_to_string(&restart_path) {
                    if token != last_restart {
                        restart_requested = true;
                        break;
                    }
                }

                // Settings capture UI writes this flag — skip hotkeys while set.
                let capturing = std::fs::metadata(
                    std::env::temp_dir().join("switchfix.capture-pause"),
                ).is_ok();
                if capturing {
                    // Drain any queued hotkeys so they don't fire after capture.
                    while events.try_recv().is_ok() {}
                }
            }
        }
    }

    tray_handle.abort();
    switchfix_adapters::clear_daemon_heartbeat();

    if restart_requested {
        info!("restarting daemon (settings save)");
        // Brief pause so the tray icon can disappear cleanly on KDE.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        reexec_daemon()?;
    }

    info!("daemon stopped");
    Ok(())
}

/// Replace this process with a fresh daemon (same PID — systemd stays happy).
/// Hotkeys are bound at startup, so a re-exec is the reliable way to rebind.
fn reexec_daemon() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&exe).args(&args).exec();
        Err(anyhow::anyhow!("failed to re-exec daemon: {err}"))
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(&exe).args(&args).spawn()?;
        std::process::exit(0);
    }
}
