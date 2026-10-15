//! Platform factory: compile-time OS + runtime desktop selection.

use super::stub::UnsupportedPlatform;
use super::{detect, OsKind, Platform, PlatformInfo};

/// Detect and build the right [`Platform`]. Never fails: stubs for unsupported combos.
pub fn create_platform() -> Box<dyn Platform> {
    create_platform_from(detect())
}

/// Select a [`Platform`] for a known [`PlatformInfo`] (deterministic; used by tests too).
pub fn create_platform_from(info: PlatformInfo) -> Box<dyn Platform> {
    match info.os {
        OsKind::Linux => {
            #[cfg(target_os = "linux")]
            {
                return super::linux::select(info);
            }
            #[cfg(not(target_os = "linux"))]
            {
                Box::new(UnsupportedPlatform::new(info))
            }
        }
        OsKind::Windows => {
            #[cfg(target_os = "windows")]
            {
                Box::new(super::windows::WindowsPlatform::new(info))
            }
            #[cfg(not(target_os = "windows"))]
            {
                Box::new(UnsupportedPlatform::new(info))
            }
        }
        OsKind::Macos => {
            #[cfg(target_os = "macos")]
            {
                Box::new(super::macos::MacosPlatform::new(info))
            }
            #[cfg(not(target_os = "macos"))]
            {
                Box::new(UnsupportedPlatform::new(info))
            }
        }
        OsKind::Unsupported => Box::new(UnsupportedPlatform::new(info)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{detect_from, OsKind};

    #[tokio::test]
    async fn factory_linux_kde_is_supported() {
        let info = detect_from(OsKind::Linux, Some("KDE"), Some("wayland"));
        let p = create_platform_from(info);
        let i = p.info();
        assert_eq!(i.id, "linux/kde/wayland");
        assert!(i.supported);
    }

    #[tokio::test]
    async fn factory_linux_gnome_stubs() {
        let info = detect_from(OsKind::Linux, Some("GNOME"), Some("wayland"));
        let p = create_platform_from(info);
        assert!(!p.info().supported);
        let err = p
            .build_services(&Default::default(), &Default::default())
            .await
            .err()
            .expect("gnome must not build services");
        let msg = err.to_string();
        assert!(msg.contains("unsupported"), "msg={msg}");
    }

    #[tokio::test]
    async fn factory_foreign_os_stubs() {
        for os in [OsKind::Windows, OsKind::Macos, OsKind::Unsupported] {
            let info = detect_from(os, Some(""), Some(""));
            let p = create_platform_from(info);
            assert!(!p.info().supported);
            let err = p
                .build_services(&Default::default(), &Default::default())
                .await
                .err()
                .expect("foreign OS must not build services");
            assert!(err.to_string().contains("unsupported"));
        }
    }
}
