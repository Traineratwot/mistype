//! Fallback platform used when no full implementation exists.

use switchfix_app::config::AppConfig;

use super::{BuildOptions, Platform, PlatformError, PlatformInfo, PlatformServices};

/// Reports detection info and refuses to build services.
pub struct UnsupportedPlatform {
    info: PlatformInfo,
}

impl UnsupportedPlatform {
    pub fn new(info: PlatformInfo) -> Self {
        Self { info }
    }
}

#[async_trait::async_trait]
impl Platform for UnsupportedPlatform {
    fn info(&self) -> PlatformInfo {
        self.info.clone()
    }

    async fn build_services(
        &self,
        _cfg: &AppConfig,
        _opts: &BuildOptions,
    ) -> Result<PlatformServices, PlatformError> {
        Err(PlatformError::Unsupported {
            reason: self
                .info
                .stub_reason
                .clone()
                .unwrap_or_else(|| self.info.id.clone()),
        })
    }
}
