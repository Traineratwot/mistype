//! Daemon heartbeat file: written while the main process is alive.

use std::path::PathBuf;

use mistype_app::ports::HeartbeatPort;

/// Where the heartbeat file lives.
pub fn daemon_heartbeat_path() -> PathBuf {
    std::env::temp_dir().join("mistype.daemon.json")
}

/// Payload of the daemon heartbeat file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DaemonHeartbeat {
    pub pid: u32,
    pub enabled: bool,
    /// True while an LLM-fix request is in flight (for the Plasma widget).
    #[serde(default)]
    pub busy: bool,
}

/// Record that the daemon is alive (call from the main loop / on toggle).
pub fn write_daemon_heartbeat(enabled: bool) {
    write_daemon_heartbeat_ex(enabled, false);
}

/// Heartbeat with an explicit LLM-busy flag.
pub fn write_daemon_heartbeat_ex(enabled: bool, busy: bool) {
    let hb = DaemonHeartbeat {
        pid: std::process::id(),
        enabled,
        busy,
    };
    if let Ok(json) = serde_json::to_string(&hb) {
        let _ = std::fs::write(daemon_heartbeat_path(), json);
    }
}

/// Remove the heartbeat on clean shutdown.
pub fn clear_daemon_heartbeat() {
    let _ = std::fs::remove_file(daemon_heartbeat_path());
}

/// Read heartbeat if the recorded pid is still running.
pub fn read_daemon_heartbeat() -> Option<DaemonHeartbeat> {
    let json = std::fs::read_to_string(daemon_heartbeat_path()).ok()?;
    let hb: DaemonHeartbeat = serde_json::from_str(&json).ok()?;
    if std::path::Path::new(&format!("/proc/{}", hb.pid)).is_dir() {
        Some(hb)
    } else {
        None
    }
}

// ---------- port wrapper ----------

/// [`HeartbeatPort`] over the file helpers above.
pub struct FileHeartbeat;

impl FileHeartbeat {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl HeartbeatPort for FileHeartbeat {
    fn touch(&self, enabled: bool, busy: bool) {
        write_daemon_heartbeat_ex(enabled, busy);
    }
    fn clear(&self) {
        clear_daemon_heartbeat();
    }
}

