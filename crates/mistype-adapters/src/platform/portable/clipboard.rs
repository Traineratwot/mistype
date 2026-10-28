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
    CaptureError, CaptureTarget, CapturedSelection, ClipboardError, ClipboardPort, ClipboardRead,
    SavedClipboard, SelectionCapture, TextInjector,
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

/// KDE Klipper exposes the live (Wayland) clipboard over D-Bus. Needed on
/// Plasma Wayland where `arboard` (X11/XWayland) cannot see copies made by
/// native Wayland apps — the marker protocol would never observe the copy.
fn get_text_klipper() -> Option<String> {
    if !which("dbus-send") {
        return None;
    }
    let out = Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper.getClipboardContents",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_dbus_string_reply(&String::from_utf8_lossy(&out.stdout))
}

/// Body of a `dbus-send --print-reply` message is `   string "<raw text>"`.
/// The value is printed raw (quotes and newlines are not escaped), so the
/// content is everything between the type tag and the final quote.
fn parse_dbus_string_reply(stdout: &str) -> Option<String> {
    let trimmed = stdout.trim_end();
    let rest = trimmed.split_once("string \"")?.1;
    Some(rest.strip_suffix('"')?.to_string())
}

fn put_text_klipper(text: &str) -> bool {
    if !which("dbus-send") {
        return false;
    }
    // `--print-reply` is required: without it dbus-send exits before the
    // message is flushed and the set is silently lost.
    Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.kde.klipper",
            "/klipper",
            "org.kde.klipper.klipper.setClipboardContents",
        ])
        .arg(format!("string:{text}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
                        let r = get_text_external()
                            .or_else(get_text_klipper)
                            .ok_or_else(|| {
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
                        // Prefer wl-copy (separate process, offer survives us),
                        // then Klipper (native Wayland clipboard on KDE), then
                        // arboard (X11) as the last resort.
                        let mut result = put_text_via_wl_copy(&text);
                        if result.is_err() && put_text_klipper(&text) {
                            result = Ok(());
                        }
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

/// Copy chords that are safe for the focused app.
///
/// Ctrl+Shift+C opens DevTools in browsers and Ctrl+C is SIGINT in terminals,
/// so each environment gets its own fallback and never the other's. Ctrl+Insert
/// is universally safe and always goes first. With unknown focus no fallback is
/// attempted at all — a blind chord does more harm than a missed selection.
fn copy_chords(target: CaptureTarget) -> &'static [KeyChord] {
    match target {
        CaptureTarget::Terminal => &[KeyChord::CtrlInsert, KeyChord::CtrlShiftC],
        CaptureTarget::Gui => &[KeyChord::CtrlInsert, KeyChord::CtrlC],
        CaptureTarget::Unknown => &[KeyChord::CtrlInsert],
    }
}

#[async_trait]
impl SelectionCapture for ArboardClipboard {
    /// Clipboard-marker protocol:
    /// 1. save user clipboard
    /// 2. put unique marker
    /// 3. copy selection with the chords safe for the focused app
    /// 4. read clipboard — if marker survived, there was no selection
    ///
    /// On success returns the selected text plus the saved clipboard snapshot
    /// so the caller can restore it after replacing the selection. The marker
    /// is never left in the clipboard on any abort path.
    async fn capture_selection(
        &self,
        target: CaptureTarget,
    ) -> Result<Option<CapturedSelection>, CaptureError> {
        let _g = self.lock.lock().await;

        let marker = format!("MISTYPE_MARK_{}", Uuid::new_v4());
        let saved = self.req_get().ok();
        let snap = SavedClipboard {
            text: saved.clone(),
        };
        *self.last_save.lock().unwrap() = Some(snap.clone());

        self.req_set(&marker)
            .map_err(|e| CaptureError::Failed(e.to_string()))?;

        // The marker must become visible where the copy will be read back —
        // otherwise a stale read looks like a "successful" capture of the
        // user's old clipboard content.
        let mut marker_ok = false;
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            match self.req_get() {
                Ok(t) if t == marker => {
                    marker_ok = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        if !marker_ok {
            self.discard_marker(&snap, &marker).await;
            return Err(CaptureError::Failed(
                "clipboard did not accept the capture marker".into(),
            ));
        }

        let Some(inj) = &self.injector else {
            self.discard_marker(&snap, &marker).await;
            return Err(CaptureError::Failed(
                "no injector for selection capture".into(),
            ));
        };

        // Poll instead of one fixed sleep: a late copy must not be mistaken for
        // "no selection" — that would fire the fallback chord blindly.
        let per_chord = Duration::from_millis(self.settle_ms.clamp(150, 3000));
        for chord in copy_chords(target) {
            if let Err(e) = inj.chord(*chord).await {
                debug!("copy chord {chord:?}: {e}");
            }
            let deadline = tokio::time::Instant::now() + per_chord;
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let now = match self.req_get() {
                    Ok(t) => t,
                    Err(e) => {
                        self.discard_marker(&snap, &marker).await;
                        return Err(CaptureError::Failed(e.to_string()));
                    }
                };
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
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
        }

        // No selection — restore clipboard immediately and never leave the marker.
        debug!("selection capture: no copy chord produced text (target={target:?})");
        self.discard_marker(&snap, &marker).await;
        Ok(None)
    }
}

impl ArboardClipboard {
    /// Abort path of `capture_selection`: never leave the marker in the
    /// clipboard. Restore the saved content; when it is unknown, clear the
    /// marker only if it is still the current clipboard content.
    async fn discard_marker(&self, snap: &SavedClipboard, marker: &str) {
        match &snap.text {
            Some(t) => {
                let _ = self.req_set(t);
            }
            None => {
                if let Ok(cur) = self.req_get() {
                    if cur == marker {
                        let _ = self.req_set("");
                    }
                }
            }
        }
        *self.last_save.lock().unwrap() = None;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_chords_never_mix_risky_fallbacks() {
        // Ctrl+Shift+C opens DevTools in browsers — GUI must never see it.
        let gui = copy_chords(CaptureTarget::Gui);
        assert!(gui.contains(&KeyChord::CtrlInsert));
        assert!(!gui.contains(&KeyChord::CtrlShiftC));
        assert!(gui.contains(&KeyChord::CtrlC));

        // Ctrl+C is SIGINT in terminals — terminal must never see it.
        let term = copy_chords(CaptureTarget::Terminal);
        assert!(term.contains(&KeyChord::CtrlInsert));
        assert!(!term.contains(&KeyChord::CtrlC));
        assert!(term.contains(&KeyChord::CtrlShiftC));

        // Unknown focus: only the universally safe chord.
        assert_eq!(copy_chords(CaptureTarget::Unknown), &[KeyChord::CtrlInsert]);
    }

    #[test]
    fn parses_dbus_string_reply() {
        let reply = "method return time=1 sender=:1.27 -> destination=:1.437 serial=3 reply_serial=2\n   string \"hello\"\n";
        assert_eq!(parse_dbus_string_reply(reply).as_deref(), Some("hello"));

        // Raw value: embedded newlines, tabs, quotes and a trailing quote.
        let tricky = "method return time=1\n   string \"a\nb\tc \"q\" d\"\"\n";
        assert_eq!(
            parse_dbus_string_reply(tricky).as_deref(),
            Some("a\nb\tc \"q\" d\"")
        );

        // Empty clipboard.
        assert_eq!(
            parse_dbus_string_reply("   string \"\"\n").as_deref(),
            Some("")
        );

        // Error reply has no string body.
        assert_eq!(parse_dbus_string_reply("Error: no owner\n"), None);
    }
}
