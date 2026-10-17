//! System tray icon via StatusNotifierItem (ksni) — native KDE tray.
//!
//! Flashes the icon briefly when a hotkey fires so the user can see that
//! SwitchFix received the keyboard event.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, ToolTip, Tray};
use tokio::sync::mpsc;
use tracing::info;

/// Commands the tray can emit to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleEnabled,
    ConvertSelection,
    LlmFixSelection,
    OpenSettings,
    Quit,
    /// Accept a suggestion slot (1-based).
    AcceptSuggestion(u8),
    /// Add the flagged word to the personal dictionary.
    LearnWord,
    /// Dismiss current suggestions.
    DismissSuggestions,
}

/// Shared state between daemon and tray.
pub struct TrayModel {
    pub enabled: AtomicBool,
    /// On while the icon should blink after a hotkey.
    pub flash: AtomicBool,
    /// On while the LLM is processing a selection (yellow blink).
    pub busy: AtomicBool,
    pub cmd_tx: mpsc::Sender<TrayCommand>,
    /// Current suggestion slots (labels for the menu), empty when none.
    suggestions: std::sync::Mutex<Option<(String, Vec<String>, bool)>>,
}

impl TrayModel {
    pub fn new(cmd_tx: mpsc::Sender<TrayCommand>, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            flash: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            cmd_tx,
            suggestions: std::sync::Mutex::new(None),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }

    pub fn is_flashing(&self) -> bool {
        self.flash.load(Ordering::SeqCst)
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Update the suggestion menu contents. `None` clears them.
    pub fn set_suggestions(&self, original: &str, slots: &[String], can_learn: bool) {
        *self.suggestions.lock().unwrap() = Some((
            original.to_string(),
            slots.to_vec(),
            can_learn,
        ));
    }

    pub fn clear_suggestions(&self) {
        *self.suggestions.lock().unwrap() = None;
    }

    pub fn suggestions(&self) -> Option<(String, Vec<String>, bool)> {
        self.suggestions.lock().unwrap().clone()
    }
}

pub struct SwitchFixTray {
    pub model: Arc<TrayModel>,
}

/// Handle kept so the tray stays alive. Also allows forcing a redraw (flash).
pub struct TrayHandle {
    model: Arc<TrayModel>,
    handle: ksni::Handle<SwitchFixTray>,
    _worker: Option<tokio::task::JoinHandle<()>>,
}

impl TrayHandle {
    /// Light up the tray icon for `duration` (hotkey feedback).
    pub fn flash(&self, duration: Duration) {
        self.model.flash.store(true, Ordering::SeqCst);
        self.handle.update(|_| ());

        let model = self.model.clone();
        let handle = self.handle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            model.flash.store(false, Ordering::SeqCst);
            handle.update(|_| ());
        });
    }

    /// Yellow blinking while the LLM is working on a selection.
    /// Call `set_busy(false)` when done (or `true` again to keep going).
    pub fn set_busy(&self, busy: bool) {
        self.model.busy.store(busy, Ordering::SeqCst);
        self.handle.update(|_| ());
        if !busy {
            return;
        }
        // Blink loop: redraw every 400 ms while busy so the icon alternates.
        let model = self.model.clone();
        let handle = self.handle.clone();
        tokio::spawn(async move {
            while model.is_busy() {
                tokio::time::sleep(Duration::from_millis(400)).await;
                if !model.is_busy() {
                    break;
                }
                // Toggle the flash bit to alternate yellow/gray.
                let cur = model.flash.load(Ordering::SeqCst);
                model.flash.store(!cur, Ordering::SeqCst);
                handle.update(|_| ());
            }
            model.flash.store(false, Ordering::SeqCst);
            handle.update(|_| ());
        });
    }

    /// Force a tray redraw (menu / tooltip refresh).
    pub fn refresh(&self) {
        self.handle.update(|_| ());
    }

    pub fn abort(self) {
        if let Some(h) = self._worker {
            h.abort();
        }
    }
}

fn flash_color() -> (u8, u8, u8) {
    (0xff, 0xc1, 0x07) // amber/yellow — clearly different from green/gray
}

fn busy_color() -> (u8, u8, u8) {
    (0xff, 0xd6, 0x00) // bright yellow — LLM is working
}

impl Tray for SwitchFixTray {
    fn id(&self) -> String {
        "switchfix".into()
    }

    fn title(&self) -> String {
        "SwitchFix".into()
    }

    fn icon_name(&self) -> String {
        if self.model.is_busy() {
            "view-refresh".into()
        } else if self.model.is_flashing() {
            "notification-message-im".into()
        } else if self.model.is_enabled() {
            "input-keyboard".into()
        } else {
            "media-playback-pause".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let (r, g, b) = if self.model.is_busy() {
            busy_color()
        } else if self.model.is_flashing() {
            flash_color()
        } else if self.model.is_enabled() {
            (0x2e, 0xcc, 0x71)
        } else {
            (0x88, 0x88, 0x88)
        };
        let mut data = Vec::with_capacity(16 * 16 * 4);
        for _ in 0..(16 * 16) {
            data.extend_from_slice(&[r, g, b, 0xff]);
        }
        vec![Icon {
            width: 16,
            height: 16,
            data,
        }]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: self.icon_pixmap(),
            title: "SwitchFix".into(),
            description: if self.model.is_busy() {
                "LLM исправляет текст…".into()
            } else if self.model.is_flashing() {
                "Горячая клавиша…".into()
            } else if self.model.is_enabled() {
                "Включён".into()
            } else {
                "Приостановлен".into()
            },
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items = vec![
            MenuItem::Standard(StandardItem {
                label: if self.model.is_enabled() {
                    "Приостановить SwitchFix".into()
                } else {
                    "Включить SwitchFix".into()
                },
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::ToggleEnabled);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Исправить выделенный текст".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::ConvertSelection);
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "LLM: исправить выделенное".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::LlmFixSelection);
                }),
                ..Default::default()
            }),
        ];

        // Suggestion candidates (fallback when the Plasma widget is not placed).
        if let Some((original, slots, can_learn)) = self.model.suggestions() {
            items.push(MenuItem::Separator);
            items.push(MenuItem::Standard(StandardItem {
                label: format!("Исправить: «{original}»"),
                enabled: false,
                ..Default::default()
            }));
            for (i, s) in slots.iter().enumerate() {
                let n = (i + 1) as u8;
                let label = format!("{}. {s}", i + 1);
                items.push(MenuItem::Standard(StandardItem {
                    label,
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.model.cmd_tx.try_send(TrayCommand::AcceptSuggestion(n));
                    }),
                    ..Default::default()
                }));
            }
            if can_learn {
                items.push(MenuItem::Standard(StandardItem {
                    label: "+в словарь".into(),
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.model.cmd_tx.try_send(TrayCommand::LearnWord);
                    }),
                    ..Default::default()
                }));
            }
            items.push(MenuItem::Standard(StandardItem {
                label: "Пропустить".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::DismissSuggestions);
                }),
                ..Default::default()
            }));
        }

        items.push(MenuItem::Separator);
        items.push(MenuItem::Standard(StandardItem {
            label: "Настройки…".into(),
            activate: Box::new(|this: &mut Self| {
                let _ = this.model.cmd_tx.try_send(TrayCommand::OpenSettings);
            }),
            ..Default::default()
        }));
        items.push(MenuItem::Standard(StandardItem {
            label: "Выход".into(),
            activate: Box::new(|this: &mut Self| {
                let _ = this.model.cmd_tx.try_send(TrayCommand::Quit);
            }),
            ..Default::default()
        }));
        items
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.model.cmd_tx.try_send(TrayCommand::ToggleEnabled);
    }
}

/// Spawn the tray service. Returns a handle that must be kept alive.
pub async fn spawn_tray(model: Arc<TrayModel>) -> Result<TrayHandle, String> {
    let service = ksni::TrayService::new(SwitchFixTray {
        model: model.clone(),
    });
    let handle = service.handle();
    service.spawn();
    info!("tray started");

    let keep = handle.clone();
    let worker = tokio::spawn(async move {
        let _keep = keep;
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });

    Ok(TrayHandle {
        model,
        handle,
        _worker: Some(worker),
    })
}
