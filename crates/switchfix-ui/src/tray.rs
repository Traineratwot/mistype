//! System tray icon via StatusNotifierItem (ksni) — native KDE tray.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, ToolTip, Tray};
use tokio::sync::mpsc;
use tracing::info;

/// Commands the tray can emit to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleEnabled,
    ConvertSelection,
    ConvertLastWord,
    OpenSettings,
    Quit,
}

/// Shared state between daemon and tray.
pub struct TrayModel {
    pub enabled: AtomicBool,
    pub cmd_tx: mpsc::Sender<TrayCommand>,
}

impl TrayModel {
    pub fn new(cmd_tx: mpsc::Sender<TrayCommand>, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            cmd_tx,
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }
}

pub struct SwitchFixTray {
    pub model: Arc<TrayModel>,
}

/// Handle kept so the tray stays alive.
pub struct TrayHandle {
    _worker: Option<tokio::task::JoinHandle<()>>,
}

impl TrayHandle {
    pub fn abort(self) {
        if let Some(h) = self._worker {
            h.abort();
        }
    }
}

impl Tray for SwitchFixTray {
    fn id(&self) -> String {
        "switchfix".into()
    }

    fn title(&self) -> String {
        "SwitchFix".into()
    }

    fn icon_name(&self) -> String {
        if self.model.is_enabled() {
            "input-keyboard".into()
        } else {
            "media-playback-pause".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let (r, g, b) = if self.model.is_enabled() {
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
            description: if self.model.is_enabled() {
                "Включён".into()
            } else {
                "Приостановлен".into()
            },
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::Standard(StandardItem {
                label: if self.model.is_enabled() {
                    "Приостановить".into()
                } else {
                    "Включить".into()
                },
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::ToggleEnabled);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Конвертировать выделение".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::ConvertSelection);
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Конвертировать последнее слово".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::ConvertLastWord);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Настройки…".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::OpenSettings);
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Выход".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.model.cmd_tx.try_send(TrayCommand::Quit);
                }),
                ..Default::default()
            }),
        ]
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

    let worker = tokio::spawn(async move {
        let _keep = handle;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });

    Ok(TrayHandle {
        _worker: Some(worker),
    })
}
