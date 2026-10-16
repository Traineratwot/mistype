//! Diagnostics: `switchfix doctor`

use std::path::Path;

use switchfix_adapters::detect;

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
            println!("Fix interactively (asks for sudo only when needed):");
            println!("  switchfix setup");
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
                 (log out/in, or: sg input -c switchfix daemon)"
                    .into()
            },
        ));

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
                    #[zbus(name = "getLayout")]
                    fn get_layout(&self) -> zbus::Result<u32>;
                    #[zbus(name = "getLayoutsList")]
                    fn get_layouts_list(&self) -> zbus::Result<Vec<(String, String, String)>>;
                    #[zbus(name = "setLayout")]
                    fn set_layout(&self, index: u32) -> zbus::Result<bool>;
                }
                match ProbeProxy::new(&conn).await {
                    Ok(p) => match p.get_layouts_list().await {
                        Ok(list) => {
                            let names: Vec<_> =
                                list.iter().map(|(s, _, n)| format!("{s}({n})")).collect();
                            let cur = p.get_layout().await.unwrap_or(999);
                            // Write probe: setLayout(current) is a safe no-op that
                            // verifies the set path actually works.
                            let write_ok = p.set_layout(cur).await.unwrap_or(false);
                            let write_note = if write_ok {
                                "setLayout ok"
                            } else {
                                "setLayout rejected"
                            };
                            (
                                write_ok,
                                format!(
                                    "layouts: [{}] current index {cur} ({write_note})",
                                    names.join(", ")
                                ),
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
    }

    // Config path
    let cfg_path = switchfix_adapters::TomlConfigStore::default_path();
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

    // Active hotkey bindings from config (what to actually press).
    {
        use switchfix_app::ports::{ConfigStore, SpellChecker};
        let store = switchfix_adapters::TomlConfigStore::open_default();
        let cfg = store.load().unwrap_or_default();
        lines.push((
            "hotkeys".into(),
            true,
            format!(
                "selection=[{}]  toggle=[{}]  fix-spelling=[{}]",
                cfg.hotkeys.convert_selection, cfg.hotkeys.toggle_enabled, cfg.hotkeys.fix_spelling
            ),
        ));

        // Spellcheck dictionaries + user word list.
        if cfg.spell.enabled {
            let user_dic = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("switchfix")
                .join("user.dic");
            let dict = switchfix_adapters::ZspellDictionary::load(&cfg.spell.languages, user_dic.clone());
            let langs = dict.languages();
            let has = !langs.is_empty();
            lines.push((
                "spell dictionaries".into(),
                has,
                if has {
                    format!(
                        "loaded [{}], mode={}, user.dic={}",
                        langs.join(", "),
                        cfg.spell.mode,
                        if user_dic.is_file() {
                            user_dic.display().to_string()
                        } else {
                            "(not created yet)".into()
                        }
                    )
                } else {
                    "none found — install hunspell-ru / hunspell-en-us (or switchfix spell off)".into()
                },
            ));

            // Plasma suggestion widget.
            let plasmoid_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".local/share/plasma/plasmoids/org.switchfix.suggestions");
            let has_widget = plasmoid_dir.is_dir();
            lines.push((
                "plasmoid".into(),
                has_widget,
                if has_widget {
                    "org.switchfix.suggestions installed".into()
                } else {
                    "not installed — run `switchfix setup` or copy packaging/plasmoid/org.switchfix.suggestions to ~/.local/share/plasma/plasmoids/".into()
                },
            ));
        } else {
            lines.push(("spell dictionaries".into(), true, "spellcheck disabled".into()));
        }
    }

    DoctorReport { lines }
}
