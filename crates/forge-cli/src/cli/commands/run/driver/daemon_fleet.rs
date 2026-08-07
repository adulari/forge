//! In-process `forge_core::fleet::FleetMessaging` for a forge-serve-hosted session: talks
//! directly to the daemon's own [`crate::serve::SessionRegistry`] + [`forge_store::Store`], no
//! HTTP hop. A CLI-bridge process (`forge mcp-serve`) or `forge send` — different processes, with
//! no registry access — implement the same shape over HTTP instead; see
//! `crate::mcp_serve::fleet` and `crate::cli::commands::send`.

use forge_core::fleet::{FleetMessaging, FleetPeer, MessageMode};

pub(crate) struct DaemonFleetMessaging {
    pub(crate) registry: std::sync::Arc<crate::serve::SessionRegistry>,
    pub(crate) store: std::sync::Arc<forge_store::Store>,
    pub(crate) self_id: String,
}

#[async_trait::async_trait]
impl FleetMessaging for DaemonFleetMessaging {
    async fn peers(&self) -> Vec<FleetPeer> {
        self.registry
            .all()
            .await
            .into_iter()
            .filter(|h| h.session_id != self.self_id)
            .map(|h| {
                let title = h.title();
                FleetPeer {
                    id: h.session_id.clone(),
                    title: (!title.is_empty()).then_some(title),
                }
            })
            .collect()
    }

    async fn send(
        &self,
        target_session_id: &str,
        mode: MessageMode,
        text: &str,
    ) -> Result<(), String> {
        // Read our own live title fresh (rather than caching it at construction time) so a
        // `/title` rename mid-session shows up in the next message this sender sends.
        let sender_label = self
            .registry
            .get(&self.self_id)
            .await
            .map(|h| h.title())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| self.self_id[..self.self_id.len().min(8)].to_string());
        let id = forge_types::new_id();
        self.store
            .enqueue_fleet_message(
                &id,
                "session",
                Some(&self.self_id),
                &sender_label,
                target_session_id,
                text,
                mode.as_str(),
            )
            .map_err(|e| e.to_string())?;
        crate::serve::deliver_pending_fleet_messages(
            &self.store,
            &self.registry,
            target_session_id,
        )
        .await;
        Ok(())
    }
}
