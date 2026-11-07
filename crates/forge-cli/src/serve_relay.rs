//! Brings up the fresh-worker session a goal relay created (docs/features/goal-contract.md,
//! "Fresh-worker relay"). The goal's driver writes the successor into the store and asks here; the
//! daemon hosts it through the same create-with-resume route a restart uses, and the new driver
//! starts the goal from the stored brief.

use std::sync::Arc;

use axum::extract::State;

use crate::serve::{create_session, CreateSessionReq, DaemonState};

pub(crate) fn start(state: &Arc<DaemonState>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    state.registry.set_relay_launcher(tx);
    let state = state.clone();
    tokio::spawn(async move {
        while let Some(id) = rx.recv().await {
            let request = CreateSessionReq {
                resume: Some(id.clone()),
                ..Default::default()
            };
            let response = create_session(State(state.clone()), Ok(axum::Json(request))).await;
            if response.status().is_success() {
                crate::serve_custody::successor_hosted(&state, &id).await;
            } else {
                crate::serve_custody::settle(&state);
                // The successor stays in the fleet flag set by the relay, so the next daemon
                // start retries it.
                eprintln!(
                    "⚠ could not host the fresh worker {id}: {}",
                    response.status()
                );
            }
        }
    });
}
