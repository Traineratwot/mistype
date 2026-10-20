//! Log-based notifier (desktop notifications can be added later).

use mistype_app::ports::{Notifier, NotifyError, NotifyLevel};
use tracing::{info, warn};

/// Log-based notifier (desktop notifications can be added later).
pub struct LogNotifier;

#[async_trait::async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, msg: &str, level: NotifyLevel) -> Result<(), NotifyError> {
        match level {
            NotifyLevel::Info => info!(target: "mistype", "{msg}"),
            NotifyLevel::Warning => warn!(target: "mistype", "{msg}"),
            NotifyLevel::Error => tracing::error!(target: "mistype", "{msg}"),
        }
        Ok(())
    }
}
