//! Removing a session's driver only while it is idle, decided under the registry lock.

use std::sync::Arc;

use super::SessionRegistry;
use crate::cli::commands::run::driver::SessionDriverHandle;

impl SessionRegistry {
    /// Drop `id`'s driver only if it is still idle, deciding under the registry lock so a turn
    /// that started after the caller's earlier check is never shut down under itself. `Err` carries
    /// the reason it was kept.
    pub(crate) async fn remove_if_idle(
        &self,
        id: &str,
    ) -> Result<Option<Arc<SessionDriverHandle>>, &'static str> {
        let mut sessions = self.sessions.lock().await;
        let Some(handle) = sessions.get(id) else {
            return Ok(None);
        };
        if !handle.is_idle() {
            return Err("this session started a turn while it was syncing");
        }
        let removed = sessions.remove(id);
        drop(sessions);
        self.notify_fleet();
        Ok(removed)
    }
}
