//! Daemon heartbeat file: written while the main process is alive.

use std::path::PathBuf;

/// Where the heartbeat file lives.
pub fn daemon_heartbeat_path() -> PathBuf {
    std::env::temp_dir().join("switchfix.daemon.json")
}

/// Payload of the daemon heartbeat file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DaemonHeartbeat {
    pub pid: u32,
    pub enabled: bool,
}

/// Record that the daemon is alive (call from the main loop / on toggle).
pub fn write_daemon_heartbeat(enabled: bool) {
    let hb = DaemonHeartbeat {
        pid: std::process::id(),
        enabled,
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
