//! Terminal suggestion fallback (no GUI window).
//!
//! Prints suggestions as they appear — useful when the Plasma widget is not
//! installed. Accept via `Ctrl+1/2/3` (global hotkeys) or
//! `switchfix suggest-accept N`.
//!
//! Kept free of `switchfix-adapters` (UI must not depend on adapters);
//! the IPC paths mirror `platform/portable/suggestion_state.rs`.

use std::path::PathBuf;
use std::time::Duration;

use tracing::info;

fn suggest_state_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.suggest.json")
}

/// Block and print suggestion-bar updates to stdout (CLI fallback).
pub fn run_suggest_bar() {
    info!("suggestion console fallback (no GUI window)");
    println!("SwitchFix: ожидание подсказок… (Ctrl+C для выхода)");
    println!("Принять: Ctrl+1/2/3 или `switchfix suggest-accept N`");

    let mut last = String::new();
    loop {
        match std::fs::read_to_string(suggest_state_path()) {
            Ok(json) => {
                if json != last {
                    last = json.clone();
                    print_suggestions(&json);
                }
            }
            Err(_) => {
                if !last.is_empty() {
                    last.clear();
                    println!("— скрыто —");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn print_suggestions(json: &str) {
    if json.trim().is_empty() || json.trim() == "{}" {
        return;
    }
    if let Ok(model) = serde_json::from_str::<switchfix_app::ports::SuggestionUi>(json) {
        println!("\nИсправить: «{}»", model.original);
        for (i, s) in model.slots.iter().enumerate() {
            let kind = match s.kind {
                switchfix_domain::SuggestionKind::LayoutConvert => " (раскладка)",
                switchfix_domain::SuggestionKind::SpellFix => "",
                switchfix_domain::SuggestionKind::Completion => " (дописывание)",
            };
            println!("  {}. {}{kind}", i + 1, s.text);
        }
        if model.can_add_to_dict {
            println!("  +в словарь  (Ctrl+0)");
        }
        return;
    }
    println!("{json}");
}
