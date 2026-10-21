//! One-shot update check against GitHub Releases.
//!
//! Only called from `mistype doctor` (explicit user action) — never automatic.

use std::time::Duration;

use mistype_app::ports::{UpdateChecker, UpdateStatus};

const RELEASES_URL: &str = "https://api.github.com/repos/Traineratwot/mistype/releases/latest";
const TIMEOUT: Duration = Duration::from_secs(5);

pub struct GithubUpdateChecker {
    current: String,
}

impl GithubUpdateChecker {
    pub fn new() -> Self {
        Self {
            current: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Parse `v2.1.0` / `2.1.0` into a comparable triple. `None` = non-semver tag.
    pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
        let s = s.trim().trim_start_matches('v');
        let mut parts = s.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        // Patch may carry a pre-release suffix: `0-rc1` → `0`.
        let patch_raw = parts.next()?;
        let patch: u64 = patch_raw
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or("")
            .parse()
            .ok()?;
        Some((major, minor, patch))
    }

    /// Pure comparison — unit-testable without network.
    pub fn compare(current: &str, remote_tag: &str) -> UpdateStatus {
        let remote = remote_tag.trim().trim_start_matches('v');
        match (Self::parse_version(current), Self::parse_version(remote_tag)) {
            (Some(cur), Some(rem)) => {
                if rem > cur {
                    UpdateStatus::Available(format!("v{remote}"))
                } else {
                    UpdateStatus::UpToDate(current.to_string())
                }
            }
            _ => UpdateStatus::Unavailable(format!("unparsable version: {remote_tag}")),
        }
    }

    /// Extract `tag_name` from the GitHub Releases JSON payload.
    pub fn parse_tag(body: &str) -> Result<String, String> {
        let v: serde_json::Value =
            serde_json::from_str(body).map_err(|e| format!("json: {e}"))?;
        v.get("tag_name")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "missing tag_name".into())
    }

    fn fetch_latest_tag() -> Result<String, String> {
        let req = ureq::get(RELEASES_URL)
            .timeout(TIMEOUT)
            .set("User-Agent", &format!("mistype/{}", env!("CARGO_PKG_VERSION")))
            .set("Accept", "application/vnd.github+json");
        let resp = req.call().map_err(|e| format!("{e}"))?;
        let status = resp.status();
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}"));
        }
        let body = resp.into_string().map_err(|e| format!("read body: {e}"))?;
        Self::parse_tag(&body)
    }
}

impl Default for GithubUpdateChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateChecker for GithubUpdateChecker {
    fn check(&self) -> UpdateStatus {
        match Self::fetch_latest_tag() {
            Ok(tag) => Self::compare(&self.current, &tag),
            Err(e) => UpdateStatus::Unavailable(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_strips_v_prefix() {
        assert_eq!(GithubUpdateChecker::parse_version("v2.1.0"), Some((2, 1, 0)));
        assert_eq!(GithubUpdateChecker::parse_version("2.1.0"), Some((2, 1, 0)));
        assert_eq!(GithubUpdateChecker::parse_version("v2.1.0-rc1"), Some((2, 1, 0)));
        assert_eq!(GithubUpdateChecker::parse_version("garbage"), None);
    }

    #[test]
    fn compare_detects_newer() {
        assert_eq!(
            GithubUpdateChecker::compare("2.0.0", "v2.1.0"),
            UpdateStatus::Available("v2.1.0".into())
        );
        assert_eq!(
            GithubUpdateChecker::compare("2.0.0", "v2.0.0"),
            UpdateStatus::UpToDate("2.0.0".into())
        );
        assert_eq!(
            GithubUpdateChecker::compare("2.1.0", "v2.0.0"),
            UpdateStatus::UpToDate("2.1.0".into())
        );
    }

    #[test]
    fn parse_tag_reads_tag_name() {
        assert_eq!(
            GithubUpdateChecker::parse_tag(r#"{"tag_name":"v2.1.0"}"#).unwrap(),
            "v2.1.0"
        );
        assert!(GithubUpdateChecker::parse_tag("{}").is_err());
    }
}
