//! Clipboard via `arboard` (held on a dedicated thread) + `wl-copy` fallback,
//! plus selection capture (marker protocol).
//!
//! On Wayland/X11 the clipboard is *hosted* by the process that set it. Dropping
//! the `arboard::Clipboard` right after `set_text` (or forking from a
//! multi-threaded daemon — `wl-clipboard-rs` does that) makes the offer vanish,
//! so a later Ctrl+V pastes empty and *deletes the selection*. We keep one
//! long-lived owner thread and, when available, prefer `wl-copy` which
//! daemonizes properly from a child process.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use mistype_app::ports::{
    CaptureError, CapturedSelection, ClipboardError, ClipboardPort, ClipboardRead, SavedClipboard,
    SelectionCapture, TextInjector,
};
use mistype_domain::KeyChord;
use tokio::sync::Mutex as AsyncMutex;
use tracing::debug;
use uuid::Uuid;

/// Clipboard adapter. One background thread owns `arboard::Clipboard` for the
/// process lifetime so the clipboard offer stays alive after `set_text`.
pub struct ArboardClipboard {
    lock: AsyncMutex<()>,
    injector: Option<Box<dyn TextInjector>>,
    settle_ms: u64,
    last_save: Mutex<Option<SavedClipboard>>,
    tx: mpsc::Sender<ClipReq>,
}

enum ClipReq {
    Get {
        reply: mpsc::Sender<Result<String, ClipboardError>>,
    },
    Set {
        text: String,
        reply: mpsc::Sender<Result<(), ClipboardError>>,
    },
}

fn clip_err_unavailable(e: impl std::fmt::Display) -> ClipboardError {
    ClipboardError::Unavailable(e.to_string())
}

fn clip_err_failed(e: impl std::fmt::Display) -> ClipboardError {
    ClipboardError::Failed(e.to_string())
}

fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(name).is_file()))
        .unwrap_or(false)
}

fn get_text_external() -> Option<String> {
    if !which("wl-paste") {
        return None;
    }
    let out = Command::new("wl-paste")
        .args(["--no-newline"])
        .output()
        .ok()?;
    // Empty clipboard → non-zero exit; treat as empty string.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn put_text_via_wl_copy(text: &str) -> Result<(), ClipboardError> {
    if !which("wl-copy") {
        return Err(ClipboardError::Unavailable("wl-copy not found".into()));
    }
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(clip_err_unavailable)?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClipboardError::Failed("wl-copy stdin".into()))?;
        stdin.write_all(text.as_bytes()).map_err(clip_err_failed)?;
    }
    // wl-copy forks a holder and the parent exits.
    let _ = child.wait();
    Ok(())
}

impl ArboardClipboard {
    pub fn new(injector: Option<Box<dyn TextInjector>>, settle_ms: u64) -> Self {
        let (tx, rx) = mpsc::channel::<ClipReq>();
        // Dedicated OS thread keeps a single arboard owner alive for the process
        // lifetime (Wayland/X11 serve paste requests from the owner process).
        std::thread::spawn(move || {
            let mut held: Option<arboard::Clipboard> = None;
            for req in rx {
                match req {
                    ClipReq::Get { reply } => {
                        let r = get_text_external().ok_or_else(|| {
                            ClipboardError::Unavailable("wl-paste unavailable".into())
                        });
                        let r = match r {
                            Ok(text) => Ok(text),
                            Err(_) => {
                                let cb = match held.as_mut() {
                                    Some(cb) => Ok(cb),
                                    None => match arboard::Clipboard::new() {
                                        Ok(cb) => {
                                            held = Some(cb);
                                            Ok(held.as_mut().unwrap())
                                        }
                                        Err(e) => Err(clip_err_unavailable(e)),
                                    },
                                };
                                match cb {
                                    Ok(cb) => cb.get_text().map_err(clip_err_failed),
                                    Err(e) => Err(e),
                                }
                            }
                        };
                        let _ = reply.send(r);
                    }
                    ClipReq::Set { text, reply } => {
                        // Prefer wl-copy (separate process, offer survives us).
                        let mut result = put_text_via_wl_copy(&text);
                        if result.is_err() {
                            let cb = match held.as_mut() {
                                Some(cb) => Ok(cb),
                                None => match arboard::Clipboard::new() {
                                    Ok(cb) => {
                                        held = Some(cb);
                                        Ok(held.as_mut().unwrap())
                                    }
                                    Err(e) => Err(clip_err_unavailable(e)),
                                },
                            };
                            result = match cb {
                                Ok(cb) => cb.set_text(text.clone()).map_err(clip_err_failed),
                                Err(e) => Err(e),
                            };
                        }
                        let _ = reply.send(result);
                    }
                }
            }
        });

        Self {
            lock: AsyncMutex::new(()),
            injector,
            settle_ms,
            last_save: Mutex::new(None),
            tx,
        }
    }

    fn req_get(&self) -> Result<String, ClipboardError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(ClipReq::Get { reply })
            .map_err(|_| ClipboardError::Unavailable("clipboard thread gone".into()))?;
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| ClipboardError::Unavailable("clipboard get timeout".into()))?
    }

    fn req_set(&self, text: &str) -> Result<(), ClipboardError> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(ClipReq::Set {
                text: text.to_string(),
                reply,
            })
            .map_err(|_| ClipboardError::Unavailable("clipboard thread gone".into()))?;
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| ClipboardError::Unavailable("clipboard set timeout".into()))?
    }
}

#[async_trait]
impl ClipboardPort for ArboardClipboard {
    async fn save(&self) -> Result<SavedClipboard, ClipboardError> {
        let _g = self.lock.lock().await;
        let text = self.req_get().ok();
        let snap = SavedClipboard { text };
        *self.last_save.lock().unwrap() = Some(snap.clone());
        Ok(snap)
    }

    async fn put_text(&self, text: &str) -> Result<(), ClipboardError> {
        let _g = self.lock.lock().await;
        self.req_set(text)
    }

    async fn get_text(&self) -> Result<String, ClipboardError> {
        let _g = self.lock.lock().await;
        self.req_get()
    }

    async fn restore(&self, saved: SavedClipboard) -> Result<(), ClipboardError> {
        let _g = self.lock.lock().await;
        match saved.text {
            Some(t) => self.req_set(&t),
            None => Ok(()),
        }
    }
}

/// Read-only view for the settings UI paste buttons. Does not touch
/// `last_save` / marker-protocol state.
impl ClipboardRead for ArboardClipboard {
    fn get_text(&self) -> Result<String, ClipboardError> {
        self.req_get()
    }
}

#[async_trait]
impl SelectionCapture for ArboardClipboard {
    /// Clipboard-marker protocol:
    /// 1. save user clipboard
    /// 2. put unique marker
    /// 3. copy selection (Ctrl+C, then Ctrl+Insert)
    /// 4. read clipboard — if marker survived, there was no selection
    ///
    /// On success returns the selected text plus the saved clipboard snapshot
    /// so the caller can restore it after replacing the selection.
    async fn capture_selection(&self) -> Result<Option<CapturedSelection>, CaptureError> {
        let _g = self.lock.lock().await;

        let marker = format!("MISTYPE_MARK_{}", Uuid::new_v4());
        let saved = self.req_get().ok();
        let snap = SavedClipboard {
            text: saved.clone(),
        };
        *self.last_save.lock().unwrap() = Some(snap.clone());

        self.req_set(&marker)
            .map_err(|e| CaptureError::Failed(e.to_string()))?;

        let Some(inj) = &self.injector else {
            return Err(CaptureError::Failed(
                "no injector for selection capture".into(),
            ));
        };

        // Ctrl+Insert first (safe in terminals), then Ctrl+Shift+C (terminals),
        // then Ctrl+C (GUI). Never lead with Ctrl+C — in Konsole it is SIGINT.
        for chord in [KeyChord::CtrlInsert, KeyChord::CtrlShiftC, KeyChord::CtrlC] {
            if let Err(e) = inj.chord(chord).await {
                debug!("copy chord {chord:?}: {e}");
            }
            tokio::time::sleep(Duration::from_millis(self.settle_ms.max(120) / 2)).await;
            let now = self
                .req_get()
                .map_err(|e| CaptureError::Failed(e.to_string()))?;
            if now != marker && !now.is_empty() {
                debug!(
                    "selection captured via {chord:?} ({} chars)",
                    now.chars().count()
                );
                return Ok(Some(CapturedSelection {
                    text: now,
                    saved: snap,
                }));
            }
        }

        // No selection — restore clipboard immediately.
        if let Some(s) = &saved {
            let _ = self.req_set(s);
        }
        *self.last_save.lock().unwrap() = None;
        Ok(None)
    }
}

impl ArboardClipboard {
    /// Restore the snapshot saved during `capture_selection`.
    pub async fn restore_last(&self) -> Result<(), ClipboardError> {
        let saved = self.last_save.lock().unwrap().take();
        if let Some(s) = saved {
            self.restore(s).await
        } else {
            Ok(())
        }
    }
}
