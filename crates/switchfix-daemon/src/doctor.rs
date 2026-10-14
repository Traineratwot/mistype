//! Diagnostics: `switchfix doctor`

use std::path::Path;

pub struct DoctorReport {
    pub lines: Vec<(String, bool, String)>,
}

impl DoctorReport {
    pub fn print(&self) {
        println!("SwitchFix doctor");
        println!("================");
        for (name, ok, detail) in &self.lines {
            let mark = if *ok { "OK " } else { "FAIL" };
            println!("[{mark}] {name}: {detail}");
        }
        let failed = self.lines.iter().filter(|(_, ok, _)| !ok).count();
        if failed > 0 {
            println!("\n{failed} check(s) failed.");
            println!("Hint: add yourself to the 'input' group and re-login:");
            println!("  sudo usermod -aG input $USER");
        } else {
            println!("\nAll checks passed.");
        }
    }

    pub fn all_ok(&self) -> bool {
        self.lines.iter().all(|(_, ok, _)| *ok)
    }
}

pub async fn run() -> DoctorReport {
    let mut lines = Vec::new();

    // Session type
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    lines.push((
        "session".into(),
        session == "wayland" || session == "x11",
        format!("XDG_SESSION_TYPE={session}"),
    ));

    // Desktop
    let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    lines.push((
        "desktop".into(),
        true,
        format!("XDG_CURRENT_DESKTOP={de}"),
    ));

    // /dev/uinput
    let uinput_ok = Path::new("/dev/uinput").exists();
    let uinput_writable = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    lines.push((
        "uinput".into(),
        uinput_ok && uinput_writable,
        if !uinput_ok {
            "/dev/uinput missing".into()
        } else if uinput_writable {
            "/dev/uinput writable".into()
        } else {
            "/dev/uinput exists but not writable (need group 'input' or udev rule)".into()
        },
    ));

    // /dev/input keyboards
    let mut kb_count = 0;
    if let Ok(rd) = std::fs::read_dir("/dev/input") {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            if n.starts_with("event") {
                kb_count += 1;
            }
        }
    }
    lines.push((
        "input devices".into(),
        kb_count > 0,
        format!("{kb_count} /dev/input/event* nodes"),
    ));

    // input group
    let in_input_group = std::fs::read_to_string("/proc/self/status")
        .map(|s| {
            // fallback: check groups via id
            let _ = s;
            false
        })
        .unwrap_or(false);
    let groups = std::process::Command::new("id")
        .arg("-nG")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let has_input = groups.split_whitespace().any(|g| g == "input");
    lines.push((
        "group input".into(),
        has_input,
        if has_input {
            "user is in 'input' group".into()
        } else {
            format!("not in 'input' group (id -nG: {groups})")
        },
    ));
    let _ = in_input_group;

    // D-Bus org.kde.keyboard
    let layouts_ok = match zbus::Connection::session().await {
        Ok(conn) => {
            use zbus::proxy;
            #[proxy(
                interface = "org.kde.KeyboardLayouts",
                default_service = "org.kde.keyboard",
                default_path = "/Layouts"
            )]
            trait Probe {
                fn get_layout(&self) -> zbus::Result<u32>;
                fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;
            }
            match ProbeProxy::new(&conn).await {
                Ok(p) => match p.get_layouts_list().await {
                    Ok(list) => {
                        let names: Vec<_> = list.iter().map(|(s, _, n)| format!("{s}({n})")).collect();
                        let cur = p.get_layout().await.unwrap_or(999);
                        (
                            true,
                            format!("layouts: [{}] current index {cur}", names.join(", ")),
                        )
                    }
                    Err(e) => (false, format!("getLayoutsList failed: {e}")),
                },
                Err(e) => (false, format!("proxy failed: {e}")),
            }
        }
        Err(e) => (false, format!("session bus: {e}")),
    };
    lines.push(("org.kde.keyboard".into(), layouts_ok.0, layouts_ok.1));

    // Config path
    let cfg_path = switchfix_adapters::config_store::TomlConfigStore::default_path();
    lines.push((
        "config path".into(),
        true,
        cfg_path.display().to_string(),
    ));

    DoctorReport { lines }
}
