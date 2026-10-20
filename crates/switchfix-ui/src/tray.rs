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
}

/// Shared state between daemon and tray.
pub struct TrayModel {
    pub enabled: AtomicBool,
    /// On while the icon should blink after a hotkey.
    pub flash: AtomicBool,
    /// On while the LLM is processing a selection (yellow blink).
    pub busy: AtomicBool,
    pub cmd_tx: mpsc::Sender<TrayCommand>,
}

impl TrayModel {
    pub fn new(cmd_tx: mpsc::Sender<TrayCommand>, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            flash: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            cmd_tx,
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
            // Program logo (assets/icons/switchfix.svg), not the generic keyboard.
            "switchfix".into()
        } else {
            "media-playback-pause".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        // Fallback when the theme has no `switchfix` icon: compact EN↔RU logo.
        let (bg, left, right) = if self.model.is_busy() {
            (busy_color(), (0x88, 0x66, 0x00), (0x88, 0x66, 0x00))
        } else if self.model.is_flashing() {
            (flash_color(), (0x99, 0x44, 0x00), (0x99, 0x44, 0x00))
        } else if self.model.is_enabled() {
            ((0x2b, 0x2e, 0x3b), (0x5b, 0x9c, 0xff), (0xff, 0x7a, 0x59))
        } else {
            ((0x44, 0x44, 0x44), (0x77, 0x77, 0x77), (0x77, 0x77, 0x77))
        };
        let size = 16;
        let mut data = vec![0u8; size * size * 4];
        for y in 0..size {
            for x in 0..size {
                let i = (y * size + x) * 4;
                // Dark rounded-ish plate.
                let mut px = bg;
                // Left key block (EN).
                if (2..=6).contains(&x) && (4..=10).contains(&y) {
                    px = left;
                }
                // Right key block (RU).
                if (9..=13).contains(&x) && (4..=10).contains(&y) {
                    px = right;
                }
                // Swap bar in the middle.
                if (6..=9).contains(&x) && y == 7 {
                    px = (0xe8, 0xea, 0xf0);
                }
                data[i] = px.0;
                data[i + 1] = px.1;
                data[i + 2] = px.2;
                data[i + 3] = 0xff;
            }
        }
        vec![Icon {
            width: size as i32,
            height: size as i32,
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
