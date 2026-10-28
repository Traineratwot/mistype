//! Diagnostics: `mistype doctor`

use std::path::Path;

use mistype_adapters::detect;

pub struct DoctorReport {
    pub lines: Vec<(String, bool, String)>,
}

impl DoctorReport {
    pub fn print(&self) {
        println!("Mistype doctor (v{})", env!("CARGO_PKG_VERSION"));
        println!("================");
        for (name, ok, detail) in &self.lines {
            let mark = if *ok { "OK " } else { "FAIL" };
            println!("[{mark}] {name}: {detail}");
        }
        let failed = self.lines.iter().filter(|(_, ok, _)| !ok).count();
        if failed > 0 {
            println!("\n{failed} check(s) failed.");
            println!("Fix interactively (asks for sudo only when needed):");
            println!("  mistype setup");
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

    // Platform (OS / desktop / session) — selected by the platform layer.
    let info = detect();
    lines.push((
        "platform".into(),
        info.supported,
        if info.supported {
            format!("{} (supported)", info.id)
        } else {
            format!(
                "{} (stub: {})",
                info.id,
                info.stub_reason.as_deref().unwrap_or("unsupported")
            )
        },
    ));

    #[cfg(target_os = "linux")]
    {
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

        // input group — check both the current session and the passwd database.
        let groups_session = std::process::Command::new("id")
            .arg("-nG")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let user = std::env::var("USER").unwrap_or_default();
        let groups_passwd = std::process::Command::new("id")
            .arg("-nG")
            .arg(&user)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_else(|| groups_session.clone());
        let has_input_session = groups_session.split_whitespace().any(|g| g == "input");
        let has_input_passwd = groups_passwd.split_whitespace().any(|g| g == "input");
        let (input_ok, input_detail) = match (has_input_passwd, has_input_session) {
            (true, true) => (true, "user is in 'input' group".into()),
            (true, false) => (
                true,
                "user is in 'input' group (log out and back in for this session)".into(),
            ),
            (false, _) => (
                false,
                format!("not in 'input' group (id -nG {user}: {groups_passwd})"),
            ),
        };
        lines.push(("group input".into(), input_ok, input_detail));

        // Can the session actually open keyboards? (group input must be active)
        let mut can_open_kb = 0usize;
        if let Ok(rd) = std::fs::read_dir("/dev/input") {
            for e in rd.flatten() {
                let n = e.file_name();
                let n = n.to_string_lossy();
                if !n.starts_with("event") {
                    continue;
                }
                if std::fs::OpenOptions::new().read(true).open(e.path()).is_ok() {
                    can_open_kb += 1;
                }
            }
        }
        lines.push((
            "open keyboards".into(),
            can_open_kb > 0,
            if can_open_kb > 0 {
                format!("{can_open_kb} keyboard(s) openable")
            } else {
                "cannot open /dev/input/event* — session lacks group 'input' \
                 (log out/in, or: sg input -c mistype daemon)"
                    .into()
            },
        ));

        // D-Bus org.kde.keyboard (write-probe via adapters).
        let (layouts_ok, layouts_detail) = mistype_adapters::probe_layouts().await;
        lines.push(("org.kde.keyboard".into(), layouts_ok, layouts_detail));
    }

    // Config path
    let cfg_path = mistype_adapters::TomlConfigStore::default_path();
    lines.push((
        "config path".into(),
        true,
        cfg_path.display().to_string(),
    ));

    // Sound player for hotkey feedback (optional but recommended).
    let sound_ok = ["canberra-gtk-play", "paplay", "pw-play"]
        .iter()
        .any(|b| std::process::Command::new(b).arg("--help").output().is_ok());
    lines.push((
        "sound player".into(),
        sound_ok,
        if sound_ok {
            "ok (canberra-gtk-play / paplay / pw-play)".into()
        } else {
            "none found — hotkey will only blink the tray icon".into()
        },
    ));

    // Clipboard backend used by the selection-capture marker protocol.
    // On Wayland arboard talks X11/XWayland and cannot see copies made by
    // native Wayland apps — the capture would never observe the copy.
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland");
        let has_wl_copy = std::process::Command::new("wl-copy")
            .arg("--version")
            .output()
            .is_ok();
        let has_klipper = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--dest=org.kde.klipper",
                "/klipper",
                "org.kde.klipper.klipper.getClipboardContents",
            ])
            .output()
            .is_ok_and(|o| o.status.success());
        let (ok, detail) = if has_wl_copy {
            (true, "wl-copy (native clipboard)".to_string())
        } else if has_klipper {
            (true, "klipper (native Wayland clipboard)".to_string())
        } else if wayland {
            (
                false,
                "x11 (arboard) on a Wayland session — selection capture cannot \
                 see native Wayland copies. Fix: sudo apt install wl-clipboard"
                    .to_string(),
            )
        } else {
            (true, "x11 (arboard)".to_string())
        };
        lines.push(("clipboard backend".into(), ok, detail));
    }

    // Active hotkey bindings from config (what to actually press).
    {
        use mistype_app::ports::ConfigStore;
        let store = mistype_adapters::TomlConfigStore::open_default();
        let cfg = store.load().unwrap_or_default();
        lines.push((
            "hotkeys".into(),
            true,
            format!(
                "selection=[{}]  toggle=[{}]  llm-fix=[{}]{}",
                cfg.hotkeys.convert_selection,
                cfg.hotkeys.toggle_enabled,
                cfg.hotkeys.llm_fix_selection,
                if cfg.llm.enabled {
                    ""
                } else {
                    " (llm disabled)"
                }
            ),
        ));

        // LLM API configuration (never print the token).
        if cfg.llm.enabled && !cfg.llm.base_url.trim().is_empty() {
            lines.push((
                "llm api".into(),
                true,
                format!("configured (base={}, model={:?})", cfg.llm.base_url, cfg.llm.model),
            ));
            // Live chat probe — FAIL is a real defect (feature on, endpoint broken).
            use mistype_app::ports::LlmProbe;
            let probe = mistype_adapters::OpenAiCorrector::new(&cfg.llm);
            match probe.test_chat(&cfg.llm).await {
                Ok(reply) => {
                    let snippet: String = reply.chars().take(40).collect();
                    lines.push((
                        "llm chat".into(),
                        true,
                        format!("endpoint replied ({snippet:?})"),
                    ));
                }
                Err(e) => {
                    lines.push(("llm chat".into(), false, format!("{e}")));
                }
            }
        } else {
            lines.push((
                "llm api".into(),
                true,
                format!(
                    "not configured (enabled={}, base_url set={})",
                    cfg.llm.enabled,
                    !cfg.llm.base_url.trim().is_empty()
                ),
            ));
        }
    }

    // Update check — informational only: offline is not a product defect.
    {
        use mistype_app::ports::UpdateChecker;
        let checker = mistype_adapters::GithubUpdateChecker::new();
        let detail = match checker.check() {
            mistype_app::ports::UpdateStatus::UpToDate(v) => format!("up to date ({v})"),
            mistype_app::ports::UpdateStatus::Available(tag) => {
                format!("update available: {tag}")
            }
            mistype_app::ports::UpdateStatus::Unavailable(reason) => {
                format!("check skipped ({reason})")
            }
        };
        lines.push(("update".into(), true, detail));
    }

    DoctorReport { lines }
}
