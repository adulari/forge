//! In-process `forge_core::dispatch::SessionDispatch` for a coordinator hosted by `forge serve`:
//! records a proposal straight into the daemon's store through the same function the
//! `POST /api/dispatches/{id}/proposal` route uses, so a direct-API coordinator and a CLI-bridge
//! coordinator (which calls that route over HTTP) cannot record a proposal differently.

use forge_core::dispatch::{dispatch_status, DispatchPlan, ProposalReceipt, SessionDispatch};

pub(crate) struct DaemonSessionDispatch {
    pub(crate) registry: std::sync::Arc<crate::serve::SessionRegistry>,
    pub(crate) store: std::sync::Arc<forge_store::Store>,
    pub(crate) coordinator_id: String,
}

/// Make `session` a coordinator when it was started as one, or when it is being resumed while
/// the dispatch it coordinates still needs it — without the second case a daemon restart would
/// bring the coordinator back unable to revise its split.
pub(super) fn wire(
    session: &mut forge_core::Session,
    registry: &std::sync::Arc<crate::serve::SessionRegistry>,
    started_as_coordinator: bool,
    resumed: bool,
) {
    let session_id = session.session_id().to_string();
    let coordinates = started_as_coordinator
        || (resumed
            && session
                .store
                .dispatch_for_coordinator(&session_id)
                .ok()
                .flatten()
                .is_some_and(|d| {
                    matches!(
                        d.status.as_str(),
                        dispatch_status::PLANNING
                            | dispatch_status::PROPOSED
                            | dispatch_status::RUNNING
                    )
                }));
    if coordinates {
        session.set_session_dispatch(Some(std::sync::Arc::new(DaemonSessionDispatch {
            registry: registry.clone(),
            store: session.store.clone(),
            coordinator_id: session_id,
        })));
    }
}

#[async_trait::async_trait]
impl SessionDispatch for DaemonSessionDispatch {
    fn max_items(&self) -> usize {
        self.store
            .dispatch_for_coordinator(&self.coordinator_id)
            .ok()
            .flatten()
            .map_or(forge_core::dispatch::DEFAULT_MAX_ITEMS, |d| {
                d.max_items.max(1) as usize
            })
    }

    async fn propose(&self, plan: DispatchPlan) -> Result<ProposalReceipt, String> {
        let dispatch = self
            .store
            .dispatch_for_coordinator(&self.coordinator_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no dispatch is recorded for this coordinator session".to_string())?;
        let row = crate::serve_dispatch::record_proposal(&self.store, &dispatch.id, |_| Ok(plan))
            .await
            .map_err(crate::serve_dispatch::DispatchError::into_message)?;
        self.registry.notify_fleet();
        Ok(ProposalReceipt {
            dispatch_id: row.id,
            items: row.items.len(),
        })
    }
}
