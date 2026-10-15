//! OS / desktop / session detection.

use super::{DesktopKind, OsKind, PlatformInfo, SessionKind};

/// Parse `XDG_CURRENT_DESKTOP` (colon-separated list, e.g. `"KDE"`, `"ubuntu:GNOME"`).
pub fn parse_desktop(raw: &str) -> DesktopKind {
    let mut saw_any = false;
    for seg in raw.split(':') {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        saw_any = true;
        if seg.eq_ignore_ascii_case("kde") || seg.eq_ignore_ascii_case("plasma") {
            return DesktopKind::Kde;
        }
        if seg.eq_ignore_ascii_case("gnome") {
            return DesktopKind::Gnome;
        }
    }
    if !saw_any {
        return DesktopKind::Unknown;
    }
    // Second pass: GNOME-like ids that are not exactly "GNOME" (e.g. "ubuntu:GNOME"
    // already matched; "GNOME-Classic" would land here).
    let lower = raw.to_ascii_lowercase();
    if lower.contains("gnome") {
        return DesktopKind::Gnome;
    }
    if lower.contains("kde") || lower.contains("plasma") {
        return DesktopKind::Kde;
    }
    DesktopKind::Other
}

/// Parse `XDG_SESSION_TYPE` (`"wayland"` | `"x11"`).
pub fn parse_session(raw: &str) -> SessionKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wayland" => SessionKind::Wayland,
        "x11" => SessionKind::X11,
        _ => SessionKind::Unknown,
    }
}

/// Compile-time OS detection.
pub fn detect_os() -> OsKind {
    if cfg!(target_os = "linux") {
        OsKind::Linux
    } else if cfg!(target_os = "windows") {
        OsKind::Windows
    } else if cfg!(target_os = "macos") {
        OsKind::Macos
    } else {
        OsKind::Unsupported
    }
}

fn os_str(os: OsKind) -> &'static str {
    match os {
        OsKind::Linux => "linux",
        OsKind::Windows => "windows",
        OsKind::Macos => "macos",
        OsKind::Unsupported => "unsupported",
    }
}

fn desktop_str(d: DesktopKind) -> &'static str {
    match d {
        DesktopKind::Kde => "kde",
        DesktopKind::Gnome => "gnome",
        DesktopKind::Other => "other",
        DesktopKind::Unknown => "unknown",
    }
}

fn session_str(s: SessionKind) -> &'static str {
    match s {
        SessionKind::Wayland => "wayland",
        SessionKind::X11 => "x11",
        SessionKind::Unknown => "unknown",
    }
}

/// Pure detection from raw env values (unit-testable, no env mutation).
pub fn detect_from(
    os: OsKind,
    raw_desktop: Option<&str>,
    raw_session: Option<&str>,
) -> PlatformInfo {
    let desktop = parse_desktop(raw_desktop.unwrap_or(""));
    let session = parse_session(raw_session.unwrap_or(""));
    let id = format!(
        "{}/{}/{}",
        os_str(os),
        desktop_str(desktop),
        session_str(session)
    );

    let supported = os == OsKind::Linux && desktop == DesktopKind::Kde;
    let stub_reason = if supported {
        None
    } else if os != OsKind::Linux {
        Some(format!(
            "OS '{}' is not implemented yet (only Linux/KDE Plasma is supported)",
            os_str(os)
        ))
    } else if desktop == DesktopKind::Gnome {
        Some(
            "GNOME is not implemented yet (planned: XDG Desktop Portal / gsettings). Use KDE Plasma for now."
                .to_string(),
        )
    } else {
        Some(format!(
            "desktop '{}' is not supported yet (only KDE Plasma)",
            desktop_str(desktop)
        ))
    };

    PlatformInfo {
        os,
        desktop,
        session,
        id,
        supported,
        stub_reason,
    }
}

/// Reads `XDG_CURRENT_DESKTOP` / `XDG_SESSION_TYPE` from the environment.
pub fn detect() -> PlatformInfo {
    detect_from(
        detect_os(),
        std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref(),
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_desktop_kde() {
        assert_eq!(parse_desktop("KDE"), DesktopKind::Kde);
        assert_eq!(parse_desktop("kde"), DesktopKind::Kde);
        assert_eq!(parse_desktop("KDE:plasma"), DesktopKind::Kde);
    }

    #[test]
    fn parse_desktop_gnome() {
        assert_eq!(parse_desktop("GNOME"), DesktopKind::Gnome);
        assert_eq!(parse_desktop("ubuntu:GNOME"), DesktopKind::Gnome);
        assert_eq!(parse_desktop("gnome"), DesktopKind::Gnome);
    }

    #[test]
    fn parse_desktop_other_and_unknown() {
        assert_eq!(parse_desktop("X-Cinnamon"), DesktopKind::Other);
        assert_eq!(parse_desktop("Hyprland"), DesktopKind::Other);
        assert_eq!(parse_desktop(""), DesktopKind::Unknown);
    }

    #[test]
    fn parse_session_values() {
        assert_eq!(parse_session("wayland"), SessionKind::Wayland);
        assert_eq!(parse_session("x11"), SessionKind::X11);
        assert_eq!(parse_session("tty"), SessionKind::Unknown);
    }

    #[test]
    fn detect_from_linux_kde() {
        let info = detect_from(OsKind::Linux, Some("KDE"), Some("wayland"));
        assert_eq!(info.id, "linux/kde/wayland");
        assert!(info.supported);
        assert!(info.stub_reason.is_none());
    }

    #[test]
    fn detect_from_linux_gnome_unsupported() {
        let info = detect_from(OsKind::Linux, Some("GNOME"), Some("wayland"));
        assert_eq!(info.id, "linux/gnome/wayland");
        assert!(!info.supported);
        let reason = info.stub_reason.unwrap_or_default();
        assert!(reason.contains("GNOME"), "reason={reason}");
    }

    #[test]
    fn detect_from_windows_unsupported() {
        let info = detect_from(OsKind::Windows, Some(""), Some(""));
        assert!(!info.supported);
        assert!(info.stub_reason.is_some());
    }
}
