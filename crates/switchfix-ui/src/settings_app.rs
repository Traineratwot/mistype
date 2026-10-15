//! Settings window (iced 0.13 free-function API). Separate process from daemon.
//!
//! Hotkeys are bound by click-to-capture: press the button, then press the
//! desired key combination. Escape cancels.

use iced::keyboard::{Key, Modifiers};
use iced::widget::{
    button, checkbox, column, container, mouse_area, row, scrollable, text, tooltip,
};
use iced::{Element, Length, Subscription, Task, Theme};
use switchfix_app::config::AppConfig;
use switchfix_app::ports::{ConfigError, ConfigStore};
use tracing::info;

pub struct SettingsApp {
    cfg: AppConfig,
    store: Box<dyn ConfigStore + Send + Sync>,
    status: String,
    /// Which hotkey field is currently waiting for a key combination.
    capturing: Option<HotkeyField>,
    /// Forced modifiers per hotkey (checkboxes next to that combo only).
    mods_selection: HotkeyMods,
    mods_toggle: HotkeyMods,
    /// Daemon pause flag written while capturing so hotkeys don't fire.
    paused_for_capture: bool,
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
    Language(String),
    StartCapture(HotkeyField),
    CancelCapture,
    ClearHotkey(HotkeyField),
    KeyPressed(Key, Modifiers),
    /// Toggle a modifier checkbox for one specific hotkey.
    SetMod(HotkeyField, ModKey, bool),
    Save,
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

fn update(app: &mut SettingsApp, message: Message) -> Task<Message> {
    match message {
        Message::ToggleEnabled(v) => app.cfg.general.enabled_on_start = v,
        Message::ToggleTypingBuffer(v) => app.cfg.privacy.typing_buffer = v,
        Message::ToggleHotkeyConsume(v) => app.cfg.hotkeys.hotkey_consume = v,
        Message::Language(s) => app.cfg.general.language = s,
        Message::StartCapture(field) => {
            if !app.paused_for_capture {
                write_capture_pause(true);
                app.paused_for_capture = true;
            }
            // Seed checkboxes from the current combo so they match the binding.
            let current = match field {
                HotkeyField::Selection => app.cfg.hotkeys.convert_selection.clone(),
                HotkeyField::Toggle => app.cfg.hotkeys.toggle_enabled.clone(),
            };
            let m = mods_from_combo(&current);
            match field {
                HotkeyField::Selection => app.mods_selection = m,
                HotkeyField::Toggle => app.mods_toggle = m,
            }
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
            match field {
                HotkeyField::Selection => {
                    app.cfg.hotkeys.convert_selection = String::new();
                    app.mods_selection = HotkeyMods::default();
                }
                HotkeyField::Toggle => {
                    app.cfg.hotkeys.toggle_enabled = String::new();
                    app.mods_toggle = HotkeyMods::default();
                }
            }
            app.capturing = None;
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            app.status = "Клавиша сброшена (действие отключено)".into();
        }
        Message::SetMod(field, key, on) => {
            let mods = match field {
                HotkeyField::Selection => &mut app.mods_selection,
                HotkeyField::Toggle => &mut app.mods_toggle,
            };
            match key {
                ModKey::Ctrl => mods.ctrl = on,
                ModKey::Alt => mods.alt = on,
                ModKey::Shift => mods.shift = on,
                ModKey::Meta => mods.meta = on,
            }
            // Rewrite the stored combo with the same base key + new modifiers.
            let current = match field {
                HotkeyField::Selection => app.cfg.hotkeys.convert_selection.clone(),
                HotkeyField::Toggle => app.cfg.hotkeys.toggle_enabled.clone(),
            };
            if let Some(base) = combo_base_key(&current) {
                let combo = combo_from_mods(mods, base);
                match field {
                    HotkeyField::Selection => app.cfg.hotkeys.convert_selection = combo.clone(),
                    HotkeyField::Toggle => app.cfg.hotkeys.toggle_enabled = combo.clone(),
                }
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
            let forced = match field {
                HotkeyField::Selection => app.mods_selection,
                HotkeyField::Toggle => app.mods_toggle,
            };
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
            match field {
                HotkeyField::Selection => app.cfg.hotkeys.convert_selection = combo.clone(),
                HotkeyField::Toggle => app.cfg.hotkeys.toggle_enabled = combo.clone(),
            }
            // Keep checkboxes in sync with what was just bound.
            let m = mods_from_combo(&combo);
            match field {
                HotkeyField::Selection => app.mods_selection = m,
                HotkeyField::Toggle => app.mods_toggle = m,
            }
            app.capturing = None;
            if app.paused_for_capture {
                write_capture_pause(false);
                app.paused_for_capture = false;
            }
            app.status = format!("Установлено: {}", pretty_hotkey(&combo));
        }
        Message::Save => match app.store.save(&app.cfg) {
            Ok(()) => {
                app.status = "Сохранено. Перезапустите демон, чтобы хоткеи вступили в силу."
                    .into();
                info!("config saved from settings UI");
            }
            Err(ConfigError::Io(e)) | Err(ConfigError::Parse(e)) => {
                app.status = format!("Ошибка: {e}");
            }
        },
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
    if app.capturing.is_some() {
        iced::keyboard::on_key_press(|key, modifiers| Some(Message::KeyPressed(key, modifiers)))
    } else {
        Subscription::none()
    }
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

/// Per-hotkey row: title, capture button, and its own modifier checkboxes.
fn hotkey_row<'a>(
    title: &'a str,
    hint: &'a str,
    value: &'a str,
    field: HotkeyField,
    capturing: bool,
    mods: HotkeyMods,
) -> Element<'a, Message> {
    let btn_label = if capturing {
        "… нажмите сочетание (Esc — отмена)".to_string()
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
            .width(Length::Fixed(220.0))
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

    let label = text(title).width(Length::Fixed(200.0));

    tooltip(
        column![
            row![label, capture].spacing(12).align_y(iced::Alignment::Center),
            row![text("").width(Length::Fixed(200.0)), mod_boxes]
                .spacing(12)
                .align_y(iced::Alignment::Center),
        ]
        .spacing(4),
        container(text(hint).size(13))
            .padding(10)
            .width(Length::Fixed(340.0))
            .style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .into()
}

fn view(app: &SettingsApp) -> Element<'_, Message> {
    let cfg = &app.cfg;
    let capturing = app.capturing;

    let general = column![
        text("Общие").size(20),
        checkbox("Включать при старте", cfg.general.enabled_on_start)
            .on_toggle(Message::ToggleEnabled),
        row![
            text("Язык UI: "),
            iced::widget::text_input("ru", &cfg.general.language)
                .on_input(Message::Language)
                .width(Length::Fixed(80.0))
        ]
        .spacing(8),
    ]
    .spacing(8);

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

    let hotkeys = column![
        text("Что делает SwitchFix").size(20),
        text(
            "Исправляет текст, набранный не той раскладкой: ghbdtn → привет. \
             Нажмите на комбинацию и задайте клавиши. \
             Галочки модификаторов — свои для каждого сочетания."
        )
        .size(13),
        iced::widget::Space::with_height(8.0),
        text("Горячие клавиши").size(16),
        text("Нажмите на поле комбинации, затем — желаемые клавиши.").size(12),
        hotkey_row(
            "Исправить выделенный текст",
            "Выделите текст мышкой и нажмите сочетание — \
             выделение заменится на исправленное, а раскладка переключится. \
             Пример: выделить «ghbdtn» → станет «привет». \
             В терминале (konsole, kitty, …) SwitchFix сам очищает строку \
             и вставляет через Ctrl+Shift+V.",
            &cfg.hotkeys.convert_selection,
            HotkeyField::Selection,
            capturing == Some(HotkeyField::Selection),
            app.mods_selection,
        ),
        hotkey_row(
            "Пауза / продолжить",
            "Временно выключает SwitchFix: хоткеи не срабатывают, \
             буфер набранных символов очищается. Повторное нажатие включает обратно.",
            &cfg.hotkeys.toggle_enabled,
            HotkeyField::Toggle,
            capturing == Some(HotkeyField::Toggle),
            app.mods_toggle,
        ),
        cancel_btn,
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
                     Требует группу input и перезапуск демона."
                )
                .size(13)
            )
            .padding(10)
            .width(Length::Fixed(340.0))
            .style(container::rounded_box),
            tooltip::Position::Bottom,
        ),
        text("Изменения хоткеев вступают в силу после перезапуска демона.").size(12),
    ]
    .spacing(10);

    let privacy = column![
        text("Приватность").size(20),
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
            .width(Length::Fixed(340.0))
            .style(container::rounded_box),
            tooltip::Position::Bottom,
        ),
        text("SwitchFix не отправляет данные в сеть и не сохраняет введённый текст на диск.")
            .size(12),
    ]
    .spacing(8);

    let buttons = row![
        button("Сохранить").on_press(Message::Save),
        button("Закрыть").on_press(Message::Close),
        text(&app.status).size(14),
    ]
    .spacing(12);

    let content = column![general, hotkeys, privacy, buttons]
        .spacing(24)
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
pub fn run_settings(store: Box<dyn ConfigStore + Send + Sync>) -> iced::Result {
    // Hybrid-GPU + Wayland: the Vulkan surface can come up with an empty
    // present-mode list (dmabuf import failure on NVIDIA), which panics wgpu.
    // GL/Mesa is reliable on KDE Wayland — prefer it unless the user overrode.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "gl");
    }

    let cfg = store.load().unwrap_or_default();
    let mods_selection = mods_from_combo(&cfg.hotkeys.convert_selection);
    let mods_toggle = mods_from_combo(&cfg.hotkeys.toggle_enabled);
    let app = SettingsApp {
        cfg,
        store,
        status: String::new(),
        capturing: None,
        mods_selection,
        mods_toggle,
        paused_for_capture: false,
    };

    iced::application("SwitchFix — настройки", update, view)
        .theme(theme)
        .subscription(subscription)
        .window_size((580.0, 680.0))
        .run_with(move || (app, Task::none()))
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
