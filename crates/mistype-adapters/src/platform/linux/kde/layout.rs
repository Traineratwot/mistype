//! KDE Plasma keyboard layout switcher over D-Bus (`org.kde.keyboard`).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use mistype_app::ports::{LayoutError, LayoutSwitcher};
use mistype_domain::{LayoutId, LayoutIndex, LayoutInfo};
use tokio::sync::broadcast;
use tracing::{debug, warn};
use zbus::proxy;

#[proxy(
    interface = "org.kde.KeyboardLayouts",
    default_service = "org.kde.keyboard",
    default_path = "/Layouts"
)]
trait KeyboardLayouts {
    /// Returns current layout index (0-based).
    /// Plasma uses camelCase D-Bus names — zbus would otherwise PascalCase them.
    #[zbus(name = "getLayout")]
    fn get_layout(&self) -> zbus::Result<u32>;
    /// Returns list of (short_id, ?, display_name).
    #[zbus(name = "getLayoutsList")]
    fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;
    /// Set layout by index; returns success.
    #[zbus(name = "setLayout")]
    fn set_layout(&self, index: u32) -> zbus::Result<bool>;
    #[zbus(name = "switchToNextLayout")]
    fn switch_to_next_layout(&self) -> zbus::Result<()>;
    #[zbus(name = "switchToPreviousLayout")]
    fn switch_to_previous_layout(&self) -> zbus::Result<()>;

    #[zbus(signal, name = "layoutChanged")]
    fn layout_changed(&self, index: u32) -> zbus::Result<()>;

    #[zbus(signal, name = "layoutListChanged")]
    fn layout_list_changed(&self) -> zbus::Result<()>;
}

pub struct KdeLayoutSwitcher {
    conn: zbus::Connection,
    cached: AtomicU32,
    tx: broadcast::Sender<LayoutIndex>,
}

impl KdeLayoutSwitcher {
    pub async fn connect() -> Result<Self, LayoutError> {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        let proxy = KeyboardLayoutsProxy::new(&conn)
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;

        let current = proxy
            .get_layout()
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;

        let (tx, _) = broadcast::channel(16);

        let this = Self {
            conn,
            cached: AtomicU32::new(current),
            tx,
        };

        // Spawn listener for layoutChanged signal.
        this.spawn_signal_listener();

        Ok(this)
    }

    fn spawn_signal_listener(&self) {
        let conn = self.conn.clone();
        let tx = self.tx.clone();
        let cached = Arc::new(AtomicU32::new(self.cached.load(Ordering::SeqCst)));
        // We cannot easily share AtomicU32 with self after move; use tx only.
        let _ = cached;

        tokio::spawn(async move {
            let Ok(proxy) = KeyboardLayoutsProxy::new(&conn).await else {
                warn!("failed to create layout proxy for signal listener");
                return;
            };
            let mut stream = match proxy.receive_layout_changed().await {
                Ok(s) => s,
                Err(e) => {
                    warn!("layoutChanged subscribe failed: {e}");
                    return;
                }
            };
            use futures_lite::stream::StreamExt;
            while let Some(sig) = stream.next().await {
                if let Ok(args) = sig.args() {
                    debug!("layoutChanged -> {}", args.index());
                    let _ = tx.send(LayoutIndex(*args.index()));
                }
            }
        });
    }
}

#[async_trait]
impl LayoutSwitcher for KdeLayoutSwitcher {
    async fn current(&self) -> Result<LayoutIndex, LayoutError> {
        let proxy = KeyboardLayoutsProxy::new(&self.conn)
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        let idx = proxy
            .get_layout()
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        self.cached.store(idx, Ordering::SeqCst);
        Ok(LayoutIndex(idx))
    }

    async fn set(&self, index: LayoutIndex) -> Result<(), LayoutError> {
        let proxy = KeyboardLayoutsProxy::new(&self.conn)
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        let ok = proxy
            .set_layout(index.as_u32())
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        if !ok {
            return Err(LayoutError::Rejected(index.as_u32()));
        }
        // Confirm (poll briefly) — Plasma may apply asynchronously.
        for _ in 0..10 {
            let cur = proxy
                .get_layout()
                .await
                .map_err(|e| LayoutError::Dbus(e.to_string()))?;
            if cur == index.as_u32() {
                self.cached.store(cur, Ordering::SeqCst);
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        warn!("layout {} set but not confirmed", index.as_u32());
        self.cached.store(index.as_u32(), Ordering::SeqCst);
        Ok(())
    }

    async fn layouts(&self) -> Result<Vec<LayoutInfo>, LayoutError> {
        let proxy = KeyboardLayoutsProxy::new(&self.conn)
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        let list = proxy
            .get_layouts_list()
            .await
            .map_err(|e| LayoutError::Dbus(e.to_string()))?;
        Ok(list
            .into_iter()
            .enumerate()
            .map(|(i, (short, _mid, name))| LayoutInfo {
                index: LayoutIndex(i as u32),
                id: LayoutId::new(short),
                name,
            })
            .collect())
    }

    fn subscribe(&self) -> broadcast::Receiver<LayoutIndex> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live smoke against a real KDE session. Ignored by default.
    /// Run: `cargo test -p mistype-adapters -- --ignored`
    #[tokio::test]
    #[ignore = "needs a live KDE Plasma session bus (org.kde.keyboard)"]
    async fn live_kde_layout_roundtrip() {
        let sw = KdeLayoutSwitcher::connect().await.expect("connect");
        let before = sw.current().await.expect("current");
        let list = sw.layouts().await.expect("layouts");
        assert!(!list.is_empty(), "no layouts configured in KDE?");
        eprintln!("layouts={list:?} current={before:?}");

        // Switch to another index and back (safe even with one layout).
        let target = list
            .iter()
            .find(|l| l.index != before)
            .map(|l| l.index)
            .unwrap_or(before);
        sw.set(target).await.expect("set");
        let mid = sw.current().await.expect("current after set");
        assert_eq!(mid, target, "set() did not stick");

        sw.set(before).await.expect("restore");
        let after = sw.current().await.expect("current after restore");
        assert_eq!(after, before, "failed to restore original layout");
    }
}
