//! GNOME platform stub — groundwork for a future implementation.

use switchfix_app::config::AppConfig;

use crate::platform::{BuildOptions, Platform, PlatformError, PlatformInfo, PlatformServices};

pub struct GnomePlatform {
    info: PlatformInfo,
}

impl GnomePlatform {
    pub fn new(info: PlatformInfo) -> Self {
        Self { info }
    }
}

#[async_trait::async_trait]
impl Platform for GnomePlatform {
    fn info(&self) -> PlatformInfo {
        self.info.clone()
    }

    async fn build_services(
        &self,
        _cfg: &AppConfig,
        _opts: &BuildOptions,
    ) -> Result<PlatformServices, PlatformError> {
        Err(PlatformError::Unsupported {
            reason: "GNOME is not implemented yet (planned: XDG Desktop Portal / gsettings). \
                     Use KDE Plasma for now."
                .to_string(),
        })
    }
}
