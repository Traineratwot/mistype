//! Windows platform stub — groundwork for a future implementation.

use switchfix_app::config::AppConfig;

use crate::platform::{BuildOptions, Platform, PlatformError, PlatformInfo, PlatformServices};

pub struct WindowsPlatform {
    info: PlatformInfo,
}

impl WindowsPlatform {
    pub fn new(info: PlatformInfo) -> Self {
        Self { info }
    }
}

#[async_trait::async_trait]
impl Platform for WindowsPlatform {
    fn info(&self) -> PlatformInfo {
        self.info.clone()
    }

    async fn build_services(
        &self,
        _cfg: &AppConfig,
        _opts: &BuildOptions,
    ) -> Result<PlatformServices, PlatformError> {
        Err(PlatformError::Unsupported {
            reason: "Windows is not implemented yet (planned: SendInput / ImmGetLayout)."
                .to_string(),
        })
    }
}
