//! macOS platform stub — groundwork for a future implementation.

use switchfix_app::config::AppConfig;

use crate::platform::{BuildOptions, Platform, PlatformError, PlatformInfo, PlatformServices};

pub struct MacosPlatform {
    info: PlatformInfo,
}

impl MacosPlatform {
    pub fn new(info: PlatformInfo) -> Self {
        Self { info }
    }
}

#[async_trait::async_trait]
impl Platform for MacosPlatform {
    fn info(&self) -> PlatformInfo {
        self.info.clone()
    }

    async fn build_services(
        &self,
        _cfg: &AppConfig,
        _opts: &BuildOptions,
    ) -> Result<PlatformServices, PlatformError> {
        Err(PlatformError::Unsupported {
            reason: "macOS is not implemented yet (planned: CGEvent / TIS).".to_string(),
        })
    }
}
