//! Floating suggestion bar (separate iced process).
//!
//! Reads `/tmp/switchfix.suggest.json` (written by the daemon) and shows a
//! compact strip of candidates. Accept is via click (writes
//! `/tmp/switchfix.suggest.cmd`) or the global `Ctrl+1..N` hotkeys handled
//! by the daemon. Does not steal keyboard focus from the focused app.

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{button, center, container, mouse_area, row, text};
use iced::{Element, Length, Subscription, Task, Theme};
use switchfix_app::ports::SuggestionUi;
use tracing::info;

fn suggest_state_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.suggest.json")
}

fn suggest_cmd_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.suggest.cmd")
}

pub struct SuggestBar {
    model: Option<SuggestionUi>,
    /// Hide after this instant when set.
    hide_at: Option<std::time::Instant>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Accept(u8),
    Learn,
    Dismiss,
}

fn load_model() -> Option<SuggestionUi> {
    let path = suggest_state_path();
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_cmd(cmd: &str) {
    let _ = std::fs::write(suggest_cmd_path(), cmd);
}

fn update(app: &mut SuggestBar, message: Message) -> Task<Message> {
    match message {
        Message::Tick => {
            // Auto-hide when the daemon removed the file or the timeout expired.
            let now = std::time::Instant::now();
            if let Some(at) = app.hide_at {
                if now >= at {
                    app.model = None;
                    app.hide_at = None;
                    return Task::none();
                }
            }
            match load_model() {
                Some(m) => {
                    let changed = app.model.as_ref() != Some(&m);
                    if changed {
                        app.hide_at = Some(now + Duration::from_millis(m.timeout_ms.max(500)));
                        app.model = Some(m);
                    }
                }
                None => {
                    if app.model.is_some() {
                        app.model = None;
                        app.hide_at = None;
                    }
                }
            }
            Task::none()
        }
        Message::Accept(n) => {
            write_cmd(&format!("accept:{n}"));
            app.model = None;
            app.hide_at = None;
            Task::none()
        }
        Message::Learn => {
            write_cmd("add");
            app.model = None;
            app.hide_at = None;
            Task::none()
        }
        Message::Dismiss => {
            write_cmd("dismiss");
            app.model = None;
            app.hide_at = None;
            Task::none()
        }
    }
}

fn slot_label(n: u8, text_str: &str) -> String {
    format!("{n}. {text_str}")
}

fn view(app: &SuggestBar) -> Element<'_, Message> {
    let Some(model) = &app.model else {
        // Empty strip — keep the window alive but invisible-ish.
        return center(text("")).into();
    };

    let mut items = row![].spacing(8).align_y(iced::Alignment::Center);

    items = items.push(
        text(format!("«{}»", model.original))
            .size(13)
            .style(|_| text::Style {
                color: Some(iced::Color::from_rgba(1.0, 1.0, 1.0, 0.55)),
            }),
    );

    for (i, slot) in model.slots.iter().enumerate() {
        let n = (i + 1) as u8;
        let kind_hint = match slot.kind {
            switchfix_domain::SuggestionKind::LayoutConvert => "раскладка",
            switchfix_domain::SuggestionKind::SpellFix => "",
            switchfix_domain::SuggestionKind::Completion => "",
        };
        let label = if kind_hint.is_empty() {
            slot_label(n, &slot.text)
        } else {
            format!("{}. {} ({})", n, slot.text, kind_hint)
        };
        items = items.push(
            button(text(label).size(14))
                .padding(6)
                .on_press(Message::Accept(n)),
        );
    }

    if model.can_add_to_dict {
        items = items.push(
            button(text("+в словарь").size(12))
                .padding(6)
                .on_press(Message::Learn),
        );
    }
    items = items.push(
        button(text("✕").size(12))
            .padding(6)
            .on_press(Message::Dismiss),
    );

    let strip = container(items.padding(10))
        .width(Length::Fill)
        .center_x(Length::Fill);

    // Click anywhere outside a button dismisses (via the ✕ / timeout).
    mouse_area(strip).into()
}

fn theme(_app: &SuggestBar) -> Theme {
    Theme::Dark
}

fn subscription(_app: &SuggestBar) -> Subscription<Message> {
    iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
}

/// Launch the suggestion bar (blocking).
pub fn run_suggest_bar() -> iced::Result {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "gl");
    }
    info!("suggestion bar starting");
    iced::application("SwitchFix", update, view)
        .theme(theme)
        .subscription(subscription)
        .window_size((720.0, 56.0))
        .run_with(|| (SuggestBar { model: None, hide_at: None }, Task::none()))
}
