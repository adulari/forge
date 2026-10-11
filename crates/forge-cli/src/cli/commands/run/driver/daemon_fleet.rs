//! In-process `forge_core::fleet::FleetMessaging` for a forge-serve-hosted session: talks
//! directly to the daemon's own [`crate::serve::SessionRegistry`] + [`forge_store::Store`], no
//! HTTP hop, through the same `crate::fleet_peers` rules the HTTP routes use. A CLI-bridge
//! process (`forge mcp-serve`) has no registry access and implements the same shape over HTTP
//! instead; see `crate::mcp_serve::fleet`.

use forge_core::fleet::{FleetMessaging, ListFilter, MessageMode, SendReceipt, SessionListing};

use crate::fleet_peers::{self, Sender};

pub(crate) struct DaemonFleetMessaging {
    pub(crate) registry: std::sync::Arc<crate::serve::SessionRegistry>,
    pub(crate) store: std::sync::Arc<forge_store::Store>,
}

#[async_trait::async_trait]
impl FleetMessaging for DaemonFleetMessaging {
    async fn list(&self, me: &str, filter: &ListFilter) -> SessionListing {
        fleet_peers::list_all(&self.store, &self.registry, Some(me), filter).await
    }

    async fn send(
        &self,
        me: &str,
        address: &str,
        mode: MessageMode,
        text: &str,
    ) -> Result<SendReceipt, String> {
        // Read our own live title fresh (rather than caching it at construction time) so a
        // `/title` rename mid-session shows up in the next message this sender sends.
        let label = self
            .registry
            .get(me)
            .await
            .map(|h| h.title())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| me[..me.len().min(8)].to_string());
        let sender = Sender {
            kind: "session",
            id: Some(me.to_string()),
            label,
        };
        fleet_peers::send_from(&self.store, &self.registry, &sender, address, mode, text).await
    }
}
