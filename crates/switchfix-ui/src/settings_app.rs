//! Settings window (iced 0.13 free-function API). Separate process from daemon.
//!
//! Hotkeys are bound by click-to-capture: press the button, then press the
//! desired key combination. Escape cancels.

use std::sync::Arc;
use std::time::Duration;

use iced::keyboard::{Key, Modifiers};
use iced::widget::{
    button, checkbox, column, container, mouse_area, row, scrollable, text, tooltip,
};
use iced::{Element, Length, Subscription, Task, Theme};
use switchfix_app::config::AppConfig;
use switchfix_app::ports::{ConfigError, ConfigStore, LlmProbe};
use tracing::info;

pub struct SettingsApp {
    cfg: AppConfig,
    store: Box<dyn ConfigStore + Send + Sync>,
    /// `~/.config/switchfix` — open in a file manager for hand edits.
    config_dir: std::path::PathBuf,
    status: String,
    /// Which hotkey field is currently waiting for a key combination.
    capturing: Option<HotkeyField>,
    /// Forced modifiers per hotkey (checkboxes next to that combo only).
    /// Indexed by `HotkeyField::index()`.
    mods: [HotkeyMods; HotkeyField::COUNT],
    /// Daemon pause flag written while capturing so hotkeys don't fire.
    paused_for_capture: bool,
    /// Optional LLM connection probe (list models + test chat).
    probe: Option<Arc<dyn LlmProbe>>,
    /// Model ids fetched from `GET /v1/models` — shown as clickable hints.
    models: Vec<String>,
    /// Last LLM test / models result for the status line.
    llm_status: String,
    /// True while a test/models request is in flight.
    llm_busy: bool,
    /// Spinner animation frame (braille dots).
    spin_frame: usize,
    /// Bumps on every models fetch so stale results are dropped.
    models_gen: u64,
}

/// Modifier checkboxes bound to a single hotkey (not global).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HotkeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyField {
    Selection,
    Toggle,
    LlmFixSelection,
}

impl HotkeyField {
    pub const COUNT: usize = 3;
    pub const ALL: [HotkeyField; Self::COUNT] = [
        HotkeyField::Selection,
        HotkeyField::Toggle,
        HotkeyField::LlmFixSelection,
    ];

    pub fn index(self) -> usize {
        match self {
            HotkeyField::Selection => 0,
            HotkeyField::Toggle => 1,
            HotkeyField::LlmFixSelection => 2,
        }
    }

    fn get<'a>(self, cfg: &'a AppConfig) -> &'a str {
        match self {
            HotkeyField::Selection => &cfg.hotkeys.convert_selection,
            HotkeyField::Toggle => &cfg.hotkeys.toggle_enabled,
            HotkeyField::LlmFixSelection => &cfg.hotkeys.llm_fix_selection,
        }
    }

    fn set(self, cfg: &mut AppConfig, value: String) {
        match self {
            HotkeyField::Selection => cfg.hotkeys.convert_selection = value,
            HotkeyField::Toggle => cfg.hotkeys.toggle_enabled = value,
            HotkeyField::LlmFixSelection => cfg.hotkeys.llm_fix_selection = value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKey {
    Ctrl,
    Alt,
    Shift,
    Meta,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleEnabled(bool),
    ToggleTypingBuffer(bool),
    ToggleHotkeyConsume(bool),
    ToggleLlmEnabled(bool),
    Language(String),
    LlmBaseUrl(String),
    LlmApiToken(String),
    LlmModel(String),
    LlmSystemPrompt(String),
    LlmTimeout(String),
    /// Probe the API (tiny completion) and refresh the model list.
    TestLlm,
    /// Models arrived from `GET /v1/models` (gen, result).
    ModelsLoaded(u64, Result<Vec<String>, String>),
    /// Test completion finished.
    LlmTested(Result<String, String>),
    /// Click a model hint under the model field.
    PickModel(String),
    /// Advance the busy spinner.
    SpinnerTick,
    StartCapture(HotkeyField),
    CancelCapture,
    ClearHotkey(HotkeyField),
    KeyPressed(Key, Modifiers),
    /// Toggle a modifier checkbox for one specific hotkey.
    SetMod(HotkeyField, ModKey, bool),
    Save,
    /// Open `~/.config/switchfix/` (config.toml + maps/) in a file manager.
    OpenConfigFolder,
    /// Re-read config.toml into the form and signal the daemon to apply.
    ReloadConfig,
    Close,
}

/// Path used to pause the daemon while the capture UI is active.
fn capture_pause_path() -> std::path::PathBuf {
    std::env::temp_dir().join("switchfix.capture-pause")
}

fn write_capture_pause(on: bool) {
    let path = capture_pause_path();
    if on {
        let _ = std::fs::write(&path, "1");
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

fn reload_flag_path() -> std::path::PathBuf {
    std::env::temp_dir().join("switchfix.reload")
}

fn write_reload_flag() {
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = std::fs::write(reload_flag_path(), token.to_string());
}

/// Ask the running daemon to fully restart (rebind hotkeys via re-exec).
fn restart_flag_path() -> std::path::PathBuf {
    std::env::temp_dir().join("switchfix.restart")
}

fn write_restart_flag() {
    let token = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = std::fs::write(restart_flag_path(), token.to_string());
}

/// Fetch model ids for the hints row. With `debounce`, wait briefly so
/// typing in `base_url` does not spam the API on every keystroke.
fn fetch_models_task(app: &mut SettingsApp, debounce: bool) -> Task<Message> {
    let Some(probe) = app.probe.clone() else {
        return Task::none();
    };
    let base = app.cfg.llm.base_url.trim().to_string();
    if base.is_empty() {
        app.models.clear();
        app.models_gen = app.models_gen.wrapping_add(1);
        app.llm_busy = false;
        return Task::none();
    }
    app.models_gen = app.models_gen.wrapping_add(1);
    let gen = app.models_gen;
    let token = app.cfg.llm.api_token.clone();
    let timeout = app.cfg.llm.timeout_ms;
    if !debounce {
        app.llm_busy = true;
    }
    Task::future(async move {
        if debounce {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        let result = probe
            .list_models(&base, &token, timeout)
            .await
            .map_err(|e| e.to_string());
        Message::ModelsLoaded(gen, result)
    })
}

/// Tiny chat completion + refresh models.
fn test_llm_task(app: &mut SettingsApp) -> Task<Message> {
    let Some(probe) = app.probe.clone() else {
        app.llm_status = "Проба недоступна в этой сборке".into();
        return Task::none();
    };
    let cfg = app.cfg.llm.clone();
    let models_task = fetch_models_task(app, false);
    let test_task = Task::future(async move {
        let result = probe
            .test_chat(&cfg)
            .await
            .map(|s| {
                let s = s.trim();
                let snippet: String = s.chars().take(80).collect();
                snippet
            })
            .map_err(|e| e.to_string());
        Message::LlmTested(result)
    });
    Task::batch([test_task, models_task])
}

fn update(app: &mut SettingsApp, message: Message) -> Task<Message> {
    match message {
        Message::ToggleEnabled(v) => app.cfg.general.enabled_on_start = v,
        Message::ToggleTypingBuffer(v) => app.cfg.privacy.typing_buffer = v,
        Message::ToggleHotkeyConsume(v) => app.cfg.hotkeys.hotkey_consume = v,
        Message::ToggleLlmEnabled(v) => app.cfg.llm.enabled = v,
        Message::Language(s) => app.cfg.general.language = s,
        Message::LlmBaseUrl(s) => {
            app.cfg.llm.base_url = s;
            // Auto-refresh model hints after the user pauses typing.
            return fetch_models_task(app, true);
        }
        Message::LlmApiToken(s) => {
            app.cfg.llm.api_token = s;
            return fetch_models_task(app, true);
        }
        Message::LlmModel(s) => app.cfg.llm.model = s,
        Message::LlmSystemPrompt(s) => app.cfg.llm.system_prompt = s,
        Message::LlmTimeout(s) => {
            if let Ok(n) = s.trim().parse::<u64>() {
                app.cfg.llm.timeout_ms = n;
            }
        }
        Message::PickModel(id) => {
            app.cfg.llm.model = id.clone();
            app.status = format!("Модель: {id}");
        }
        Message::SpinnerTick => {
            app.spin_frame = app.spin_frame.wrapping_add(1);
        }
        Message::TestLlm => {
            app.llm_busy = true;
            app.llm_status = "Запрос к LLM…".into();
            return test_llm_task(app);
        }
        Message::ModelsLoaded(gen, result) => {
            if gen != app.models_gen {
                return Task::none(); // stale
            }
            app.llm_busy = false;
            match result {
                Ok(list) => {
                    app.llm_status = if list.is_empty() {
                        "Модели: список пуст".into()
                    } else {
                        format!("Модели загружены: {}", list.len())
                    };
                    app.models = list;
                }
                Err(e) => {
                    app.models.clear();
                    app.llm_status = format!("Модели: {e}");
                }
            }
        }
        Message::LlmTested(result) => {
            app.llm_busy = false;
            match result {
                Ok(snippet) => {
                    app.llm_status = format!("Тест OK: {snippet}");
                    app.status = "LLM API отвечает".into();
                }
                Err(e) => {
                    app.llm_status = format!("Тест FAILED: {e}");
                    app.status = "LLM API не отвечает — проверьте адрес и токен".into();
                }
            }
        }
        Message::StartCapture(field) => {
            if !app.paused_for_capture {
                write_capture_pause(true);
                app.paused_for_capture = true;
            }
            // Seed checkboxes from the current combo so they match the binding.
            let current = field.get(&app.cfg).to_string();
            let m = mods_from_combo(&current);
            app.mods[field.index()] = m;
            app.capturing = Some(field);
            app.status =
                "Нажмите сочетание… (Esc — сбросить клавишу, кнопка «Отмена» — выйти)".into();
        }
        Message::CancelCapture => {
            app.capturing = None;
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            app.status = "Ввод отменён".into();
        }
        Message::ClearHotkey(field) => {
            field.set(&mut app.cfg, String::new());
            app.mods[field.index()] = HotkeyMods::default();
            app.capturing = None;
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            app.status = "Клавиша сброшена (действие отключено)".into();
        }
        Message::SetMod(field, key, on) => {
            let mods = &mut app.mods[field.index()];
            match key {
                ModKey::Ctrl => mods.ctrl = on,
                ModKey::Alt => mods.alt = on,
                ModKey::Shift => mods.shift = on,
                ModKey::Meta => mods.meta = on,
            }
            let mods = app.mods[field.index()];
            // Rewrite the stored combo with the same base key + new modifiers.
            let current = field.get(&app.cfg).to_string();
            if let Some(base) = combo_base_key(&current) {
                let combo = combo_from_mods(&mods, base);
                field.set(&mut app.cfg, combo.clone());
                app.status = format!("Установлено: {}", pretty_hotkey(&combo));
            }
        }
        Message::KeyPressed(key, modifiers) => {
            let Some(field) = app.capturing else {
                return Task::none();
            };
            // Escape clears the binding (disable the action), not just cancel.
            if key == Key::Named(iced::keyboard::key::Named::Escape) {
                return update(app, Message::ClearHotkey(field));
            }
            // Merge this hotkey's checkbox modifiers with live ones.
            let forced = app.mods[field.index()];
            let mut mods = modifiers;
            if forced.ctrl {
                mods |= Modifiers::CTRL;
            }
            if forced.alt {
                mods |= Modifiers::ALT;
            }
            if forced.shift {
                mods |= Modifiers::SHIFT;
            }
            if forced.meta {
                mods |= Modifiers::LOGO;
            }
            let Some(combo) = combo_from_key(&key, mods) else {
                if !matches!(key, Key::Named(_)) || !is_modifier_key(&key) {
                    app.status = "Эта клавиша не поддерживается. \
                         Попробуйте F1–F12, Insert, CapsLock, буквы."
                        .into();
                }
                return Task::none();
            };
            field.set(&mut app.cfg, combo.clone());
            // Keep checkboxes in sync with what was just bound.
            let m = mods_from_combo(&combo);
            app.mods[field.index()] = m;
            app.capturing = None;
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            app.status = format!("Установлено: {}", pretty_hotkey(&combo));
        }
        Message::Save => match app.store.save(&app.cfg) {
            Ok(()) => {
                write_restart_flag();
                app.status = "Сохранено. Демон перезапускается…".into();
                info!("config saved from settings UI; daemon restart signaled");
            }
            Err(ConfigError::Io(e)) | Err(ConfigError::Parse(e)) => {
                app.status = format!("Ошибка: {e}");
            }
        },
        Message::OpenConfigFolder => {
            if !app.config_dir.exists() {
                let _ = std::fs::create_dir_all(&app.config_dir);
            }
            match std::process::Command::new("xdg-open").arg(&app.config_dir).spawn() {
                Ok(_) => {
                    app.status = format!("Открыт каталог: {}", app.config_dir.display());
                }
                Err(e) => {
                    app.status = format!(
                        "Не удалось открыть файловый менеджер ({e}). Путь: {}",
                        app.config_dir.display()
                    );
                }
            }
        }
        Message::ReloadConfig => {
            // Cancel capture so we don't clobber hotkey fields mid-edit.
            if app.capturing.is_some() {
                app.capturing = None;
            }
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            match app.store.load() {
                Ok(cfg) => {
                    app.cfg = cfg;
                    for f in HotkeyField::ALL {
                        let m = mods_from_combo(f.get(&app.cfg));
                        app.mods[f.index()] = m;
                    }
                    write_reload_flag();
                    app.status =
                        "Конфигурация перезагружена. Хоткеи — после перезапуска демона.".into();
                    info!("config reloaded from settings UI");
                }
                Err(ConfigError::Io(e)) | Err(ConfigError::Parse(e)) => {
                    app.status = format!("Ошибка перезагрузки: {e}");
                }
            }
        }
        Message::Close => {
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            return iced::exit();
        }
    }
    Task::none()
}

fn subscription(app: &SettingsApp) -> Subscription<Message> {
    let keys = if app.capturing.is_some() {
        iced::keyboard::on_key_press(|key, modifiers| Some(Message::KeyPressed(key, modifiers)))
    } else {
        Subscription::none()
    };
    let spin = if app.llm_busy {
        iced::time::every(Duration::from_millis(120)).map(|_| Message::SpinnerTick)
    } else {
        Subscription::none()
    };
    Subscription::batch([keys, spin])
}

fn is_modifier_key(key: &Key) -> bool {
    use iced::keyboard::key::Named;
    matches!(
        key,
        Key::Named(
            Named::Control | Named::Shift | Named::Alt | Named::Super | Named::AltGraph
        )
    )
}

/// Build a config-style combo string (`ctrl+shift+f1`) from a key press.
/// Returns `None` for modifier-only presses and unidentified keys.
fn combo_from_key(key: &Key, modifiers: Modifiers) -> Option<String> {
    // Modifier keys themselves are not a binding.
    if is_modifier_key(key) {
        return None;
    }

    let mut parts: Vec<&str> = Vec::new();
    if modifiers.control() {
        parts.push("ctrl");
    }
    if modifiers.alt() {
        parts.push("alt");
    }
    if modifiers.shift() {
        parts.push("shift");
    }
    if modifiers.logo() {
        parts.push("meta");
    }

    let name = key_name(key)?;
    parts.push(name);
    Some(parts.join("+"))
}

fn key_name(key: &Key) -> Option<&'static str> {
    use iced::keyboard::key::Named;
    match key {
        Key::Named(n) => {
            let s = match n {
                Named::F1 => "f1",
                Named::F2 => "f2",
                Named::F3 => "f3",
                Named::F4 => "f4",
                Named::F5 => "f5",
                Named::F6 => "f6",
                Named::F7 => "f7",
                Named::F8 => "f8",
                Named::F9 => "f9",
                Named::F10 => "f10",
                Named::F11 => "f11",
                Named::F12 => "f12",
                Named::F13 => "f13",
                Named::F14 => "f14",
                Named::F15 => "f15",
                Named::F16 => "f16",
                Named::F17 => "f17",
                Named::F18 => "f18",
                Named::F19 => "f19",
                Named::F20 => "f20",
                Named::Pause => "pause",
                Named::Space => "space",
                Named::Enter => "enter",
                Named::Tab => "tab",
                Named::Backspace => "backspace",
                Named::Delete => "delete",
                Named::Insert => "insert",
                Named::Home => "home",
                Named::End => "end",
                Named::PageUp => "pageup",
                Named::PageDown => "pagedown",
                Named::ArrowUp => "up",
                Named::ArrowDown => "down",
                Named::ArrowLeft => "left",
                Named::ArrowRight => "right",
                Named::Escape => "escape",
                Named::CapsLock => "capslock",
                Named::NumLock => "numlock",
                Named::ScrollLock => "scrolllock",
                Named::PrintScreen => "printscreen",
                Named::ContextMenu => "menu",
                _ => return None,
            };
            Some(s)
        }
        Key::Character(c) => {
            // Single character keys: a, b, 1, …
            let c = c.chars().next()?;
            match c.to_ascii_lowercase() {
                'a' => Some("a"),
                'b' => Some("b"),
                'c' => Some("c"),
                'd' => Some("d"),
                'e' => Some("e"),
                'f' => Some("f"),
                'g' => Some("g"),
                'h' => Some("h"),
                'i' => Some("i"),
                'j' => Some("j"),
                'k' => Some("k"),
                'l' => Some("l"),
                'm' => Some("m"),
                'n' => Some("n"),
                'o' => Some("o"),
                'p' => Some("p"),
                'q' => Some("q"),
                'r' => Some("r"),
                's' => Some("s"),
                't' => Some("t"),
                'u' => Some("u"),
                'v' => Some("v"),
                'w' => Some("w"),
                'x' => Some("x"),
                'y' => Some("y"),
                'z' => Some("z"),
                '0' => Some("0"),
                '1' => Some("1"),
                '2' => Some("2"),
                '3' => Some("3"),
                '4' => Some("4"),
                '5' => Some("5"),
                '6' => Some("6"),
                '7' => Some("7"),
                '8' => Some("8"),
                '9' => Some("9"),
                _ => None,
            }
        }
        Key::Unidentified => None,
    }
}

/// Human-readable combo: `ctrl+shift+f1` → `Ctrl+Shift+F1`.
fn pretty_hotkey(combo: &str) -> String {
    combo
        .split('+')
        .map(|p| {
            let p = p.trim();
            match p {
                "ctrl" => "Ctrl".to_string(),
                "alt" => "Alt".to_string(),
                "shift" => "Shift".to_string(),
                "meta" => "Meta".to_string(),
                "space" => "Space".to_string(),
                "pause" => "Pause".to_string(),
                "enter" => "Enter".to_string(),
                "tab" => "Tab".to_string(),
                "escape" => "Esc".to_string(),
                "backspace" => "Backspace".to_string(),
                "delete" => "Delete".to_string(),
                "insert" => "Insert".to_string(),
                "home" => "Home".to_string(),
                "end" => "End".to_string(),
                "pageup" => "PageUp".to_string(),
                "pagedown" => "PageDown".to_string(),
                "capslock" | "caps" => "CapsLock".to_string(),
                "numlock" => "NumLock".to_string(),
                "scrolllock" => "ScrollLock".to_string(),
                "printscreen" => "PrintScreen".to_string(),
                "menu" => "Menu".to_string(),
                "up" => "↑".to_string(),
                "down" => "↓".to_string(),
                "left" => "←".to_string(),
                "right" => "→".to_string(),
                s if s.len() >= 2 && s.starts_with('f') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
                    format!("F{}", &s[1..])
                }
                s if s.len() == 1 => s.to_uppercase(),
                s => s.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Parse `ctrl+alt+k` → HotkeyMods { ctrl, alt, .. }.
fn mods_from_combo(combo: &str) -> HotkeyMods {
    let mut m = HotkeyMods::default();
    for p in combo.split('+') {
        match p.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => m.ctrl = true,
            "alt" => m.alt = true,
            "shift" => m.shift = true,
            "meta" | "super" | "mod" => m.meta = true,
            _ => {}
        }
    }
    m
}

/// Last non-modifier part of a combo (`ctrl+shift+f1` → `f1`).
fn combo_base_key(combo: &str) -> Option<&str> {
    combo
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .filter(|p| {
            !matches!(
                p.to_ascii_lowercase().as_str(),
                "ctrl" | "control" | "alt" | "shift" | "meta" | "super" | "mod"
            )
        })
        .next_back()
}

/// Build combo from per-hotkey modifiers + base key name.
fn combo_from_mods(mods: &HotkeyMods, base: &str) -> String {
    let mut parts = Vec::new();
    if mods.ctrl {
        parts.push("ctrl");
    }
    if mods.alt {
        parts.push("alt");
    }
    if mods.shift {
        parts.push("shift");
    }
    if mods.meta {
        parts.push("meta");
    }
    parts.push(base);
    parts.join("+")
}

/// Semantic section card: rounded box with a title and body content.
fn section_card<'a>(
    title: &'a str,
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        column![text(title).size(18), body.into()].spacing(12),
    )
    .padding([16, 18])
    .width(Length::Fill)
    .style(container::rounded_box)
    .into()
}

/// Per-hotkey row: concrete title, short description, capture button, modifiers.
fn hotkey_row<'a>(
    title: &'a str,
    desc: &'a str,
    value: &'a str,
    field: HotkeyField,
    capturing: bool,
    mods: HotkeyMods,
) -> Element<'a, Message> {
    let btn_label = if capturing {
        "… нажмите сочетание (Esc — сброс)".to_string()
    } else if value.trim().is_empty() {
        "— не назначено —".to_string()
    } else {
        pretty_hotkey(value)
    };

    // mouse_area: no keyboard focus, so subsequent key presses reach the
    // subscription instead of being swallowed by a focused button/text_input.
    let capture = mouse_area(
        container(text(btn_label).size(14))
            .padding([8, 14])
            .width(Length::Fixed(230.0))
            .style(if capturing {
                container::rounded_box
            } else {
                container::bordered_box
            }),
    )
    .on_press(Message::StartCapture(field));

    let mod_boxes = row![
        checkbox("Ctrl", mods.ctrl).on_toggle(move |v| Message::SetMod(field, ModKey::Ctrl, v)),
        checkbox("Alt", mods.alt).on_toggle(move |v| Message::SetMod(field, ModKey::Alt, v)),
        checkbox("Shift", mods.shift).on_toggle(move |v| Message::SetMod(field, ModKey::Shift, v)),
        checkbox("Meta", mods.meta).on_toggle(move |v| Message::SetMod(field, ModKey::Meta, v)),
    ]
    .spacing(10);

    column![
        text(title).size(15),
        text(desc).size(12),
        row![capture, mod_boxes].spacing(14).align_y(iced::Alignment::Center),
    ]
    .spacing(6)
    .into()
}

fn view(app: &SettingsApp) -> Element<'_, Message> {
    let cfg = &app.cfg;
    let capturing = app.capturing;

    let cancel_btn = if capturing.is_some() {
        row![
            button("Отмена").on_press(Message::CancelCapture),
            button("Сбросить клавишу (Esc)").on_press(Message::ClearHotkey(
                capturing.unwrap_or(HotkeyField::Selection)
            )),
            text("Esc — сбросить, «Отмена» — выйти без изменений").size(12),
        ]
        .spacing(8)
    } else {
        row![].spacing(0)
    };

    // ── Общие ──────────────────────────────────────────────────────────
    let general = section_card(
        "Общие",
        column![
            checkbox("Включать SwitchFix при старте", cfg.general.enabled_on_start)
                .on_toggle(Message::ToggleEnabled),
            hotkey_row(
                "Пауза / продолжить работу",
                "Временно отключает все действия и очищает буфер ввода. \
                 Повторное нажатие включает обратно.",
                HotkeyField::Toggle.get(cfg),
                HotkeyField::Toggle,
                capturing == Some(HotkeyField::Toggle),
                app.mods[HotkeyField::Toggle.index()],
            ),
            row![
                text("Язык интерфейса:"),
                iced::widget::text_input("ru", &cfg.general.language)
                    .on_input(Message::Language)
                    .width(Length::Fixed(80.0)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(10),
    );

    // ── Текст и раскладка ──────────────────────────────────────────────
    let text_and_layout = section_card(
        "Текст и раскладка",
        column![
            text(
                "Исправляет текст, набранный не той раскладкой: ghbdtn → привет. \
                 Нажмите на поле комбинации и задайте клавиши; \
                 галочки модификаторов — свои для каждого сочетания."
            )
            .size(13),
            hotkey_row(
                "Сменить раскладку выделенного текста",
                "Выделите текст мышкой и нажмите сочетание: \
                 выделение будет переведено в другую раскладку \
                 (ghbdtn → привет), а системная раскладка переключится. \
                 В терминале (konsole, kitty, …) строка очищается \
                 и результат вставляется через Ctrl+Shift+V.",
                HotkeyField::Selection.get(cfg),
                HotkeyField::Selection,
                capturing == Some(HotkeyField::Selection),
                app.mods[HotkeyField::Selection.index()],
            ),
            cancel_btn,
        ]
        .spacing(12),
    );

    // ── LLM-исправление ────────────────────────────────────────────────
    let model_hints: Element<'_, Message> = if app.models.is_empty() {
        row![].spacing(0).into()
    } else {
        // Wrap chips into rows of 4 so the card stays readable.
        let mut rows = column![].spacing(6);
        let mut current = row![].spacing(6);
        let mut in_row = 0usize;
        for id in &app.models {
            let label = id.clone();
            let selected = app.cfg.llm.model == *id;
            let btn = button(text(label.clone()).size(12))
                .on_press(Message::PickModel(label))
                .padding([4, 8])
                .style(if selected {
                    iced::widget::button::primary
                } else {
                    iced::widget::button::secondary
                });
            current = current.push(btn);
            in_row += 1;
            if in_row == 4 {
                rows = rows.push(current);
                current = row![].spacing(6);
                in_row = 0;
            }
        }
        if in_row > 0 {
            rows = rows.push(current);
        }
        column![
            text("Доступные модели (нажмите, чтобы выбрать):").size(12),
            rows,
        ]
        .spacing(6)
        .into()
    };

    let llm = section_card(
        "LLM-исправление",
        column![
            checkbox(
                "Разрешить LLM-исправление выделенного текста",
                cfg.llm.enabled
            )
            .on_toggle(Message::ToggleLlmEnabled),
            hotkey_row(
                "Исправить выделенный текст через LLM",
                "Выделите текст и нажмите сочетание (по умолчанию Alt+R). \
                 Результат заменит выделение. Одно Backspace в течение ~4 секунд \
                 отменяет замену и возвращает исходный текст. \
                 Ручное действие, не авто-замена.",
                HotkeyField::LlmFixSelection.get(cfg),
                HotkeyField::LlmFixSelection,
                capturing == Some(HotkeyField::LlmFixSelection),
                app.mods[HotkeyField::LlmFixSelection.index()],
            ),
            row![
                text("Адрес API:"),
                iced::widget::text_input("https://api.openai.com", &cfg.llm.base_url)
                    .on_input(Message::LlmBaseUrl)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            text(
                "Базовый URL OpenAI-совместимого API. Запрос идёт на \
                 {base}/v1/chat/completions. Для Ollama: http://127.0.0.1:11434. \
                 Пусто = функция выключена."
            )
            .size(12),
            row![
                text("API-токен:"),
                iced::widget::text_input("(не задан)", &cfg.llm.api_token)
                    .on_input(Message::LlmApiToken)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            text("Токен хранится в config.toml. Пусто = без заголовка Authorization (локальные серверы).")
                .size(12),
            row![
                text("Модель:"),
                iced::widget::text_input("gpt-4o-mini", &cfg.llm.model)
                    .on_input(Message::LlmModel)
                    .width(Length::Fill),
                if app.llm_busy {
                    button("Тест…").padding([6, 12])
                } else {
                    button("Тест").on_press(Message::TestLlm).padding([6, 12])
                },
                if app.llm_busy {
                    button("Модели…").padding([6, 12])
                } else {
                    button("Модели").on_press(Message::TestLlm).padding([6, 12])
                },
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            model_hints,
            {
                const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                let spinner = if app.llm_busy {
                    text(SPIN[app.spin_frame % SPIN.len()]).size(16)
                } else {
                    text(" ").size(16)
                };
                let label = if app.llm_status.is_empty() {
                    text(" ").size(12)
                } else {
                    text(&app.llm_status).size(12)
                };
                row![spinner, label]
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
            },
            text("Системный промпт:").size(13),
            iced::widget::text_input("промпт", &cfg.llm.system_prompt)
                .on_input(Message::LlmSystemPrompt)
                .width(Length::Fill),
            text("Отправляется как system-сообщение перед выделенным текстом.")
                .size(12),
            row![
                text("Таймаут, мс:"),
                iced::widget::text_input("15000", &cfg.llm.timeout_ms.to_string())
                    .on_input(Message::LlmTimeout)
                    .width(Length::Fixed(100.0)),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(10),
    );

    // ── Перехват клавиш ────────────────────────────────────────────────
    let input_grab = section_card(
        "Перехват клавиш",
        column![
            tooltip(
                checkbox(
                    "Полностью перехватывать назначенные клавиши",
                    cfg.hotkeys.hotkey_consume
                )
                .on_toggle(Message::ToggleHotkeyConsume),
                container(
                    text(
                        "Когда включено, нажатие хоткея не доходит до других программ \
                         (клавиша «съедается»). Обычная печать продолжает работать. \
                         Требуется группа input. После сохранения демон перезапускается сам."
                    )
                    .size(13)
                )
                .padding(10)
                .width(Length::Fixed(360.0))
                .style(container::rounded_box),
                tooltip::Position::Bottom,
            ),
            text("Сохранение применяет изменения и перезапускает демон автоматически.").size(12),
        ]
        .spacing(10),
    );

    // ── Приватность ────────────────────────────────────────────────────
    let privacy = section_card(
        "Приватность",
        column![
            tooltip(
                checkbox(
                    "Запоминать последние набранные символы",
                    cfg.privacy.typing_buffer
                )
                .on_toggle(Message::ToggleTypingBuffer),
                container(
                    text(
                        "Символы хранятся только в оперативной памяти \
                         и очищаются при паузе или выходе. Отключите, \
                         если не хотите, чтобы программа запоминала ввод."
                    )
                    .size(13)
                )
                .padding(10)
                .width(Length::Fixed(360.0))
                .style(container::rounded_box),
                tooltip::Position::Bottom,
            ),
            text(
                "Текст набирается только в оперативной памяти (опционально). \
                 При включённом LLM-исправлении выделенный текст отправляется \
                 на указанный вами API-адрес. Никакой другой текст не уходит в сеть."
            )
            .size(12),
        ]
        .spacing(10),
    );

    let buttons = column![
        row![
            button("Сохранить").on_press(Message::Save),
            button("Перезагрузить конфиг").on_press(Message::ReloadConfig),
            button("Закрыть").on_press(Message::Close),
        ]
        .spacing(12),
        row![
            button("Редактировать конфиг").on_press(Message::OpenConfigFolder),
            text("config.toml и maps/ — в открывшемся каталоге").size(12),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        text(&app.status).size(14),
    ]
    .spacing(8);

    let content = column![
        text("SwitchFix — исправление текста, набранного не той раскладкой").size(22),
        text(
            "Настройте горячие клавиши: нажмите на поле комбинации, \
             затем — желаемые клавиши. Esc — сбросить действие."
        )
        .size(13),
        iced::widget::Space::with_height(4.0),
        general,
        text_and_layout,
        llm,
        input_grab,
        privacy,
        iced::widget::Space::with_height(4.0),
        buttons,
    ]
    .spacing(16)
    .padding(24)
    .width(Length::Fill);

    container(scrollable(content))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn theme(_app: &SettingsApp) -> Theme {
    Theme::Dark
}

/// Launch the settings window (blocking).
pub fn run_settings(
    store: Box<dyn ConfigStore + Send + Sync>,
    config_dir: std::path::PathBuf,
    probe: Option<Arc<dyn LlmProbe>>,
) -> iced::Result {
    // Hybrid-GPU + Wayland: the Vulkan surface can come up with an empty
    // present-mode list (dmabuf import failure on NVIDIA), which panics wgpu.
    // GL/Mesa is reliable on KDE Wayland — prefer it unless the user overrode.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "gl");
    }

    let cfg = store.load().unwrap_or_default();
    let mods = [(); HotkeyField::COUNT].map(|_| {
        // filled below from the live config
        HotkeyMods::default()
    });
    let mut mods = mods;
    for f in HotkeyField::ALL {
        mods[f.index()] = mods_from_combo(f.get(&cfg));
    }
    let mut app = SettingsApp {
        cfg,
        store,
        config_dir,
        status: String::new(),
        capturing: None,
        mods,
        paused_for_capture: false,
        probe,
        models: Vec::new(),
        llm_status: String::new(),
        llm_busy: false,
        spin_frame: 0,
        models_gen: 0,
    };

    // Auto-load model hints if the API is already configured.
    let boot = fetch_models_task(&mut app, false);

    iced::application("SwitchFix — настройки", update, view)
        .theme(theme)
        .subscription(subscription)
        .window_size((640.0, 900.0))
        .run_with(move || (app, boot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::key::Named;

    fn key_char(c: char) -> Key {
        Key::Character(c.to_string().into())
    }

    #[test]
    fn combo_ctrl_f1() {
        let m = Modifiers::CTRL;
        assert_eq!(combo_from_key(&Key::Named(Named::F1), m).as_deref(), Some("ctrl+f1"));
    }

    #[test]
    fn combo_ctrl_shift_f1() {
        let m = Modifiers::CTRL | Modifiers::SHIFT;
        assert_eq!(
            combo_from_key(&Key::Named(Named::F1), m).as_deref(),
            Some("ctrl+shift+f1")
        );
    }

    #[test]
    fn combo_ctrl_pause() {
        let m = Modifiers::CTRL;
        assert_eq!(
            combo_from_key(&Key::Named(Named::Pause), m).as_deref(),
            Some("ctrl+pause")
        );
    }

    #[test]
    fn combo_modifier_only_is_none() {
        let m = Modifiers::CTRL;
        assert_eq!(combo_from_key(&Key::Named(Named::Control), m), None);
        assert_eq!(combo_from_key(&Key::Named(Named::Shift), m), None);
    }

    #[test]
    fn combo_letter_with_ctrl() {
        let m = Modifiers::CTRL | Modifiers::ALT;
        assert_eq!(combo_from_key(&key_char('K'), m).as_deref(), Some("ctrl+alt+k"));
    }

    #[test]
    fn pretty_formats_common_combos() {
        assert_eq!(pretty_hotkey("ctrl+f1"), "Ctrl+F1");
        assert_eq!(pretty_hotkey("ctrl+shift+f1"), "Ctrl+Shift+F1");
        assert_eq!(pretty_hotkey("ctrl+pause"), "Ctrl+Pause");
        assert_eq!(pretty_hotkey("alt+space"), "Alt+Space");
    }

    #[test]
    fn mods_from_combo_parses_flags() {
        let m = mods_from_combo("ctrl+alt+k");
        assert!(m.ctrl && m.alt && !m.shift && !m.meta);
        let m = mods_from_combo("capslock");
        assert!(!m.ctrl && !m.alt && !m.shift && !m.meta);
    }

    #[test]
    fn combo_base_key_skips_modifiers() {
        assert_eq!(combo_base_key("ctrl+shift+f1"), Some("f1"));
        assert_eq!(combo_base_key("insert"), Some("insert"));
        assert_eq!(combo_base_key("ctrl+"), None);
    }

    #[test]
    fn combo_from_mods_roundtrip() {
        let m = HotkeyMods {
            ctrl: true,
            alt: false,
            shift: true,
            meta: false,
        };
        assert_eq!(combo_from_mods(&m, "f1"), "ctrl+shift+f1");
    }
}
