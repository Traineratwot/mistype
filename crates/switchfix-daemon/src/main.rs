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
    /// Play the hotkey feedback sound (and print a note) — smoke-test audio.
    Beep,
    /// Inject a synthetic hotkey (ctrl+f20) and verify evdev interception works.
    Selftest,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

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
            let tables = switchfix_domain::ConversionSet::en_ru();
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
            info!("opening settings (config at {:?})", store.path());
            let _ = cfg;
            switchfix_ui::run_settings(Box::new(store))?;
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
        Command::Daemon => {
            run_daemon().await?;
        }
    }

    Ok(())
}

async fn run_daemon() -> anyhow::Result<()> {
    info!("starting SwitchFix daemon");

    let ctx = wiring::build(wiring::WireOptions::default()).await?;

    // Tray
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(32);
    let tray_model = TrayModel::new(cmd_tx, ctx.service.is_enabled());
    let tray_handle = switchfix_ui::tray::spawn_tray(tray_model)
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

    // Also poll the toggle state file for `switchfix toggle`.
    let toggle_path = std::env::temp_dir().join("switchfix.enabled");
    let mut last_toggle = std::fs::read_to_string(&toggle_path).unwrap_or_else(|_| "1".into());

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

                        let req = match he {
                            HotkeyEvent::ConvertSelection => Some(CorrectionRequest::ConvertSelection),
                            HotkeyEvent::ConvertLastWord => Some(CorrectionRequest::ConvertLastWord),
                            HotkeyEvent::ConvertTerminal => Some(CorrectionRequest::ConvertTerminal),
                            HotkeyEvent::ToggleEnabled => None,
                        };
                        if he == HotkeyEvent::ToggleEnabled {
                            service.toggle_enabled().await;
                        } else if let Some(r) = req {
                            let out = service.handle(r).await;
                            info!("correction outcome: {out:?}");
                        }
                    }
                    None => {
                        warn!("hotkey channel closed");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {
                if let Ok(v) = std::fs::read_to_string(&toggle_path) {
                    if v != last_toggle {
                        last_toggle = v;
                        service.toggle_enabled().await;
                    }
                }
            }
        }
    }

    tray_handle.abort();
    info!("daemon stopped");
    Ok(())
}
