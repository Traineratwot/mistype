//! Log-based notifier (desktop notifications can be added later).

use switchfix_app::ports::{NotifyError, NotifyLevel, Notifier};
use tracing::{info, warn};

/// Log-based notifier (desktop notifications can be added later).
pub struct LogNotifier;

#[async_trait::async_trait]
impl Notifier for LogNotifier {
    async fn notify(&self, msg: &str, level: NotifyLevel) -> Result<(), NotifyError> {
        match level {
            NotifyLevel::Info => info!(target: "switchfix", "{msg}"),
            NotifyLevel::Warning => warn!(target: "switchfix", "{msg}"),
            NotifyLevel::Error => tracing::error!(target: "switchfix", "{msg}"),
        }
        Ok(())
    }
}
