//! Settings window (iced 0.13 free-function API). Separate process from daemon.

use iced::widget::{button, checkbox, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Task, Theme};
use switchfix_app::config::AppConfig;
use switchfix_app::ports::{ConfigError, ConfigStore};
use tracing::info;

pub struct SettingsApp {
    cfg: AppConfig,
    store: Box<dyn ConfigStore + Send + Sync>,
    status: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleEnabled(bool),
    ToggleTypingBuffer(bool),
    HotkeySelection(String),
    HotkeyWord(String),
    HotkeyTerminal(String),
    HotkeyToggle(String),
    Language(String),
    Save,
    Close,
}

fn update(app: &mut SettingsApp, message: Message) -> Task<Message> {
    match message {
        Message::ToggleEnabled(v) => app.cfg.general.enabled_on_start = v,
        Message::ToggleTypingBuffer(v) => app.cfg.privacy.typing_buffer = v,
        Message::HotkeySelection(s) => app.cfg.hotkeys.convert_selection = s,
        Message::HotkeyWord(s) => app.cfg.hotkeys.convert_last_word = s,
        Message::HotkeyTerminal(s) => app.cfg.hotkeys.convert_terminal = s,
        Message::HotkeyToggle(s) => app.cfg.hotkeys.toggle_enabled = s,
        Message::Language(s) => app.cfg.general.language = s,
        Message::Save => match app.store.save(&app.cfg) {
            Ok(()) => {
                app.status = "Сохранено".into();
                info!("config saved from settings UI");
            }
            Err(ConfigError::Io(e)) | Err(ConfigError::Parse(e)) => {
                app.status = format!("Ошибка: {e}");
            }
        },
        Message::Close => {
            return iced::exit();
        }
    }
    Task::none()
}

fn view(app: &SettingsApp) -> Element<'_, Message> {
    let cfg = &app.cfg;

    let general = column![
        text("Общие").size(20),
        checkbox("Включать при старте", cfg.general.enabled_on_start)
            .on_toggle(Message::ToggleEnabled),
        row![
            text("Язык UI: "),
            text_input("ru", &cfg.general.language)
                .on_input(Message::Language)
                .width(Length::Fixed(80.0))
        ]
        .spacing(8),
    ]
    .spacing(8);

    let hotkeys = column![
        text("Горячие клавиши").size(20),
        row![
            text("Выделение: ").width(Length::Fixed(160.0)),
            text_input("ctrl+f1", &cfg.hotkeys.convert_selection)
                .on_input(Message::HotkeySelection)
        ]
        .spacing(8),
        row![
            text("Последнее слово: ").width(Length::Fixed(160.0)),
            text_input("ctrl+f2", &cfg.hotkeys.convert_last_word)
                .on_input(Message::HotkeyWord)
        ]
        .spacing(8),
        row![
            text("Терминал: ").width(Length::Fixed(160.0)),
            text_input("ctrl+shift+f1", &cfg.hotkeys.convert_terminal)
                .on_input(Message::HotkeyTerminal)
        ]
        .spacing(8),
        row![
            text("Вкл/выкл: ").width(Length::Fixed(160.0)),
            text_input("ctrl+pause", &cfg.hotkeys.toggle_enabled)
                .on_input(Message::HotkeyToggle)
        ]
        .spacing(8),
        text("Изменения хоткеев вступают в силу после перезапуска демона.").size(12),
    ]
    .spacing(8);

    let privacy = column![
        text("Приватность").size(20),
        checkbox(
            "Хранить последние набранные символы (для Ctrl+F2)",
            cfg.privacy.typing_buffer
        )
        .on_toggle(Message::ToggleTypingBuffer),
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
    let cfg = store.load().unwrap_or_default();
    let app = SettingsApp {
        cfg,
        store,
        status: String::new(),
    };

    iced::application("SwitchFix — настройки", update, view)
        .theme(theme)
        .window_size((520.0, 560.0))
        .run_with(move || (app, Task::none()))
}
