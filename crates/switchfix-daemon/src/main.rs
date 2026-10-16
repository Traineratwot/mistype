//! SwitchFix — fix text typed in the wrong keyboard layout.

mod doctor;
mod setup;
mod wiring;

use clap::{Parser, Subcommand};
use switchfix_domain::CorrectionRequest;
use switchfix_ui::tray::TrayModel;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "switchfix", version, about = "Fix text typed in the wrong keyboard layout")]
struct Cli {
    /// Verbose logging (debug) — shows spell/hotkey decisions at runtime.
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
    /// Check spelling of a word (or --text) and print suggestions.
    Spell {
        /// Word / text to check. If omitted, reads stdin.
        #[arg(long)]
        text: Option<String>,
    },
    /// Print current suggestion-bar state as JSON (for the Plasma widget).
    SuggestPeek,
    /// Accept suggestion slot N (1-based) from the running daemon.
    SuggestAccept {
        /// Slot number 1..=3.
        slot: u8,
    },
    /// Add the flagged word to the personal dictionary.
    SuggestLearn,
    /// Dismiss the current suggestions.
    SuggestDismiss,
    /// Terminal fallback: print suggestions as they appear (no GUI window).
    SuggestBar,
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
            let _ = cfg;
            let config_dir = store
                .path()
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| {
                    dirs::config_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join("switchfix")
                });
            switchfix_ui::run_settings(Box::new(store), config_dir)?;
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
        Command::Spell { text } => {
            let input = match text {
                Some(t) => t,
                None => {
                    use std::io::Read;
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s.trim_end_matches('\n').to_string()
                }
            };
            run_spell_cli(&input);
        }
        Command::SuggestPeek => {
            // Combined status for the Plasma widget: daemon liveness + tips.
            let hb = switchfix_adapters::read_daemon_heartbeat();
            let sug = std::fs::read_to_string(switchfix_adapters::suggest_state_path())
                .ok()
                .and_then(|j| serde_json::from_str::<switchfix_app::ports::SuggestionUi>(&j).ok());
            let json = serde_json::json!({
                "daemon": hb.is_some(),
                "enabled": hb.map(|h| h.enabled).unwrap_or(false),
                "original": sug.as_ref().map(|s| s.original.clone()).unwrap_or_default(),
                "slots": sug.as_ref().map(|s| s.slots.clone()).unwrap_or_default(),
                "can_add_to_dict": sug.as_ref().map(|s| s.can_add_to_dict).unwrap_or(false),
                "timeout_ms": sug.as_ref().map(|s| s.timeout_ms).unwrap_or(0),
            });
            println!("{json}");
        }
        Command::SuggestAccept { slot } => {
            write_suggest_cmd(&format!("accept:{slot}")).await;
        }
        Command::SuggestLearn => {
            write_suggest_cmd("add").await;
        }
        Command::SuggestDismiss => {
            write_suggest_cmd("dismiss").await;
        }
        Command::SuggestBar => {
            switchfix_ui::run_suggest_bar();
        }
        Command::Daemon => {
            run_daemon().await?;
        }
    }

    Ok(())
}

/// Queue a command for the running daemon (file IPC).
async fn write_suggest_cmd(cmd: &str) {
    let path = switchfix_adapters::suggest_cmd_path();
    if let Err(e) = std::fs::write(&path, cmd) {
        eprintln!("cannot write {}: {e}", path.display());
        std::process::exit(1);
    }
    // Give the daemon a moment to pick the command up.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    println!("ok: {cmd}");
}

/// `switchfix spell --text …` — dictionary lookup without injection.
fn run_spell_cli(input: &str) {
    use switchfix_app::ports::SpellChecker;
    use switchfix_domain::{detect_lang, rank, should_check, Suggestion, SuggestionKind};

    let store = switchfix_adapters::TomlConfigStore::open_default();
    let cfg = switchfix_adapters::ensure_config(&store);
    let user_dic = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("switchfix")
        .join("user.dic");
    let dict = switchfix_adapters::ZspellDictionary::load(&cfg.spell.languages, user_dic);
    if !dict.has_any() {
        eprintln!("no spell dictionaries found (install hunspell-ru / hunspell-en-us)");
        std::process::exit(2);
    }

    let word = input.trim();
    if word.is_empty() {
        eprintln!("empty input");
        std::process::exit(2);
    }

    let Some(lang) = detect_lang(word) else {
        println!("(no letters — nothing to check)");
        return;
    };
    println!("word: {word}  lang: {lang}");

    if !should_check(word, cfg.spell.min_word_len) {
        println!("skipped (noise / too short / acronym)");
        return;
    }

    if dict.is_correct(word, lang) {
        println!("correct ✓");
        return;
    }
    println!("misspelled ✗");

    let mut candidates: Vec<Suggestion> = Vec::new();
    if let Ok(conv) = switchfix_domain::convert(
        word,
        &switchfix_domain::ConversionSet::en_ru(),
        None,
        None,
    ) {
        if let Some(cl) = detect_lang(&conv.converted) {
            if dict.is_correct(&conv.converted, cl) {
                candidates.push(Suggestion::new(
                    conv.converted,
                    SuggestionKind::LayoutConvert,
                    word,
                ));
            }
        }
    }
    for c in dict.suggest(word, lang, 8) {
        candidates.push(Suggestion::new(c, SuggestionKind::SpellFix, word));
    }
    // Obvious one-edit fixes the engine may have missed.
    for cand in switchfix_domain::one_step_neighbors(word) {
        if candidates.iter().any(|s| s.text == cand) {
            continue;
        }
        if let Some(cl) = detect_lang(&cand) {
            if dict.is_correct(&cand, cl) {
                candidates.push(Suggestion::new(cand, SuggestionKind::SpellFix, word));
            }
        }
    }
    let ranked = rank(word, candidates);
    if ranked.is_empty() {
        println!("no suggestions");
        return;
    }
    for (i, s) in ranked.iter().take(cfg.spell.suggest_count).enumerate() {
        let kind = match s.kind {
            SuggestionKind::LayoutConvert => " (раскладка)",
            SuggestionKind::SpellFix => "",
            SuggestionKind::Completion => " (дописывание)",
        };
        println!("{}. {}{kind}", i + 1, s.text);
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
        ctx.spell.clone(),
        on_settings,
        on_quit,
    );

    // Hotkey loop
    let mut events = ctx.hotkeys.events();
    let service = ctx.service.clone();
    let spell = ctx.spell.clone();
    let buffer = ctx.buffer.clone();

    // Spell word-event loop: completions + auto-fix + hints as the user types.
    let sound_cfg = ctx.config_rx.clone();
    let tray_refresh = {
        // Best-effort: keep the tray menu in sync with current suggestions.
        let model = tray_model.clone();
        std::sync::Arc::new(move || {
            let path = switchfix_adapters::suggest_state_path();
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|j| serde_json::from_str::<switchfix_app::ports::SuggestionUi>(&j).ok())
            {
                Some(ui) => {
                    let slots: Vec<String> = ui
                        .slots
                        .iter()
                        .map(|s| s.text.clone())
                        .collect();
                    model.set_suggestions(&ui.original, &slots, ui.can_add_to_dict);
                }
                None => model.clear_suggestions(),
            }
        })
    };
    if let Some(spell_svc) = spell.clone() {
        let mut word_rx = buffer.word_events();
        let tray_refresh = tray_refresh.clone();
        tokio::spawn(async move {
            while let Ok(ev) = word_rx.recv().await {
                let action = spell_svc.on_word_event(ev).await;
                tray_refresh();
                if matches!(
                    action,
                    switchfix_app::services::SpellAction::Replaced { .. }
                ) && sound_cfg.borrow().spell.sound
                {
                    switchfix_ui::play_fix_sound();
                }
            }
        });
    }

    // Suggestion UI: Plasma panel widget (org.switchfix.suggestions) and/or
    // tray menu + Ctrl+1/2/3. No floating window (it covered other apps).
    if spell.is_some() {
        info!("suggestions ready (plasmoid / tray menu / Ctrl+1..N)");
    }

    // Also poll the toggle state file for `switchfix toggle`.
    let toggle_path = std::env::temp_dir().join("switchfix.enabled");
    let mut last_toggle = std::fs::read_to_string(&toggle_path).unwrap_or_else(|_| "1".into());
    let reload_path = std::env::temp_dir().join("switchfix.reload");
    let mut last_reload = std::fs::read_to_string(&reload_path).unwrap_or_default();
    let suggest_cmd_path = switchfix_adapters::platform::portable::suggest_cmd_path();
    switchfix_adapters::write_daemon_heartbeat(service.is_enabled());

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
                            HotkeyEvent::ToggleEnabled => {
                                service.toggle_enabled().await;
                                if let Some(s) = &spell {
                                    s.set_enabled(service.is_enabled());
                                }
                                switchfix_adapters::write_daemon_heartbeat(service.is_enabled());
                            }
                            HotkeyEvent::FixSpelling => {
                                if let Some(s) = &spell {
                                    let action = s.fix_last_word().await;
                                    info!("spell fix: {action:?}");
                                    tray_refresh();
                                    tray_handle.refresh();
                                    if matches!(
                                        action,
                                        switchfix_app::services::SpellAction::Replaced { .. }
                                    ) && ctx.config_rx.borrow().spell.sound
                                    {
                                        switchfix_ui::play_fix_sound();
                                    }
                                }
                            }
                            HotkeyEvent::AcceptSuggestion(n) => {
                                if let Some(s) = &spell {
                                    let action = s.accept_slot(n).await;
                                    info!("spell accept {n}: {action:?}");
                                    tray_refresh();
                                    tray_handle.refresh();
                                    if matches!(
                                        action,
                                        switchfix_app::services::SpellAction::Replaced { .. }
                                    ) && ctx.config_rx.borrow().spell.sound
                                    {
                                        switchfix_ui::play_fix_sound();
                                    }
                                }
                            }
                            HotkeyEvent::AddToDictionary => {
                                if let Some(s) = &spell {
                                    let action = s.add_to_dictionary().await;
                                    info!("spell learn: {action:?}");
                                    tray_refresh();
                                    tray_handle.refresh();
                                }
                            }
                            HotkeyEvent::DismissSuggestions => {
                                if let Some(s) = &spell {
                                    s.dismiss().await;
                                    tray_refresh();
                                    tray_handle.refresh();
                                }
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
                        if let Some(s) = &spell {
                            s.set_enabled(service.is_enabled());
                        }
                        switchfix_adapters::write_daemon_heartbeat(service.is_enabled());
                    }
                }
                // Config / maps reload (settings button or `switchfix reload`).
                if let Ok(token) = std::fs::read_to_string(&reload_path) {
                    if token != last_reload {
                        last_reload = token.clone();
                        match ctx.store.load() {
                            Ok(cfg) => {
                                ctx.config_tx.send_replace(cfg);
                                info!("config reloaded from disk");
                            }
                            Err(e) => warn!("config reload failed, keeping previous: {e}"),
                        }
                        let tables = switchfix_adapters::load_conversion_set();
                        service.replace_tables(tables.clone());
                        if let Some(s) = &spell {
                            s.replace_tables(tables);
                        }
                        info!("layout maps reloaded (hotkeys still need a daemon restart)");
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {
                // Settings capture UI writes this flag — skip hotkeys while set.
                let capturing = std::fs::metadata(
                    std::env::temp_dir().join("switchfix.capture-pause"),
                ).is_ok();
                if capturing {
                    // Drain any queued hotkeys so they don't fire after capture.
                    while events.try_recv().is_ok() {}
                }

                // Suggestion-bar click commands (accept:N / add / dismiss).
                if let Ok(cmd) = std::fs::read_to_string(&suggest_cmd_path) {
                    let _ = std::fs::remove_file(&suggest_cmd_path);
                    let cmd = cmd.trim().to_string();
                    if let Some(s) = &spell {
                        let action = if let Some(rest) = cmd.strip_prefix("accept:") {
                            match rest.trim().parse::<u8>() {
                                Ok(n) => s.accept_slot(n).await,
                                Err(_) => switchfix_app::services::SpellAction::None,
                            }
                        } else if cmd == "add" {
                            s.add_to_dictionary().await
                        } else if cmd == "dismiss" {
                            s.dismiss().await
                        } else {
                            switchfix_app::services::SpellAction::None
                        };
                        info!("suggest-bar cmd {cmd:?}: {action:?}");
                        tray_refresh();
                        tray_handle.refresh();
                        if matches!(
                            action,
                            switchfix_app::services::SpellAction::Replaced { .. }
                        ) && ctx.config_rx.borrow().spell.sound
                        {
                            switchfix_ui::play_fix_sound();
                        }
                    }
                }

                // Auto-hide tips after bar_timeout_ms so they don't stick forever.
                if let Some(s) = &spell {
                    let timeout = ctx.config_rx.borrow().spell.bar_timeout_ms;
                    if timeout > 0 {
                        let path = switchfix_adapters::suggest_state_path();
                        let expired = std::fs::metadata(&path).ok().and_then(|m| {
                            m.modified().ok().and_then(|t| t.elapsed().ok())
                        }).map(|age| age.as_millis() as u64 >= timeout).unwrap_or(false);
                        if expired {
                            debug!("suggestion bar timeout ({timeout} ms) — hide");
                            s.dismiss().await;
                            tray_refresh();
                            tray_handle.refresh();
                        }
                    }
                }
            }
        }
    }

    tray_handle.abort();
    switchfix_adapters::clear_daemon_heartbeat();
    info!("daemon stopped");
    Ok(())
}
