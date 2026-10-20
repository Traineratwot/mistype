//! Focused-window monitor for KDE/Wayland (and X11 fallback).
//!
//! Best-effort: KWin `getWindowInfo` needs an internal id we do not always
//! have, `queryWindowInfo` is the interactive window-picker. We chain several
//! probes and return `None` when the compositor will not tell us — callers
//! then use the generic GUI path and never block.

use std::collections::HashMap;
use std::process::Command;

use async_trait::async_trait;
use mistype_app::ports::{ActiveWindow, FocusMonitor};
use tracing::debug;
use zbus::proxy;
use zbus::zvariant::OwnedValue;

#[proxy(
    interface = "org.kde.KWin",
    default_service = "org.kde.KWin",
    default_path = "/KWin"
)]
trait KWin {
    #[zbus(name = "getWindowInfo")]
    fn get_window_info(&self, internal_id: &str) -> zbus::Result<HashMap<String, OwnedValue>>;
}

pub struct KdeFocusMonitor {
    conn: zbus::Connection,
}

impl KdeFocusMonitor {
    /// Construct; `None` when the session bus is unavailable.
    pub async fn try_connect() -> Option<Self> {
        let conn = zbus::Connection::session().await.ok()?;
        Some(Self { conn })
    }
}

fn owned_str(v: &OwnedValue) -> Option<String> {
    String::try_from(v.clone()).ok()
}

fn owned_bool(v: &OwnedValue) -> Option<bool> {
    bool::try_from(v.clone()).ok()
}

fn map_to_active(map: HashMap<String, OwnedValue>) -> Option<ActiveWindow> {
    let app_id = map
        .get("resourceClass")
        .or_else(|| map.get("resource_class"))
        .or_else(|| map.get("appId"))
        .and_then(owned_str)
        .unwrap_or_default();
    let title = map
        .get("caption")
        .or_else(|| map.get("title"))
        .and_then(owned_str)
        .unwrap_or_default();
    let fullscreen = map
        .get("fullscreen")
        .or_else(|| map.get("fullScreen"))
        .and_then(owned_bool)
        .unwrap_or(false);

    if app_id.is_empty() && title.is_empty() {
        return None;
    }
    Some(ActiveWindow {
        app_id: app_id.to_ascii_lowercase(),
        title,
        fullscreen,
    })
}

/// Parse `xprop -id <id> WM_CLASS _NET_WM_STATE _NET_WM_NAME` output.
fn parse_xprop(output: &str) -> Option<ActiveWindow> {
    let mut app_id = String::new();
    let mut title = String::new();
    let mut fullscreen = false;

    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("WM_CLASS") {
            if let Some(eq) = rest.split('=').nth(1) {
                let first = eq
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .to_ascii_lowercase();
                if !first.is_empty() {
                    app_id = first;
                }
            }
        } else if line.starts_with("_NET_WM_NAME") || line.starts_with("WM_NAME") {
            if let Some(eq) = line.split('=').nth(1) {
                title = eq.trim().trim_matches('"').to_string();
            }
        } else if line.starts_with("_NET_WM_STATE") {
            fullscreen = line.contains("_NET_WM_STATE_FULLSCREEN");
        }
    }

    if app_id.is_empty() && title.is_empty() {
        return None;
    }
    Some(ActiveWindow {
        app_id,
        title,
        fullscreen,
    })
}

fn xprop_active() -> Option<ActiveWindow> {
    let root = Command::new("xprop")
        .args(["-root", "_NET_ACTIVE_WINDOW"])
        .output()
        .ok()?;
    let root_s = String::from_utf8_lossy(&root.stdout);
    let id = root_s.split('#').nth(1)?.trim().to_string();
    if id.is_empty() || id == "0x0" {
        return None;
    }
    let win = Command::new("xprop")
        .args([
            "-id",
            &id,
            "WM_CLASS",
            "_NET_WM_STATE",
            "_NET_WM_NAME",
            "WM_NAME",
        ])
        .output()
        .ok()?;
    parse_xprop(&String::from_utf8_lossy(&win.stdout))
}

#[async_trait]
impl FocusMonitor for KdeFocusMonitor {
    async fn active_window(&self) -> Option<ActiveWindow> {
        if let Some(w) = xprop_active() {
            debug!("focus via xprop: {:?}", w);
            return Some(w);
        }

        let proxy = KWinProxy::new(&self.conn).await.ok()?;
        for id in ["", "0"] {
            if let Ok(map) = proxy.get_window_info(id).await {
                if let Some(w) = map_to_active(map) {
                    debug!("focus via kwin getWindowInfo({id}): {:?}", w);
                    return Some(w);
                }
            }
        }

        debug!("focus: unknown");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xprop_konsole_fullscreen() {
        let out = r#"
WM_CLASS(STRING) = "konsole", "konsole"
_NET_WM_STATE(ATOM) = _NET_WM_STATE_FULLSCREEN, _NET_WM_STATE_FOCUSED
_NET_WM_NAME(UTF8_STRING) = "user@host: ~"
"#;
        let w = parse_xprop(out).unwrap();
        assert_eq!(w.app_id, "konsole");
        assert!(w.fullscreen);
        assert!(w.title.contains("user@host"));
    }

    #[test]
    fn parse_xprop_empty_is_none() {
        assert!(parse_xprop("").is_none());
    }
}
