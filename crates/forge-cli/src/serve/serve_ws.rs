//! The per-session WebSocket upgrade for `forge serve`.

use super::*;

pub(super) async fn ws_handler(
    State(state): State<Arc<DaemonState>>,
    axum::Extension(auth): axum::Extension<crate::serve_clients::ClientAuth>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> Response {
    match state.store.session_handoff_blocked(&params.session) {
        Ok(true) => {
            return err_response(
                axum::http::StatusCode::CONFLICT,
                "session is frozen by an Anywhere handoff",
            )
        }
        Err(error) => {
            return err_response(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("handoff state check failed: {error}"),
            )
        }
        Ok(false) => {}
    }
    // A terminal-local session with a published control channel is proxied to it rather than
    // looked up in the registry (it is not, and never will be, in there). This is what makes a
    // terminal chat drivable from the apps instead of read-only.
    if let Ok(Some(control_url)) = state.store.local_session_control_url(&params.session) {
        if !(auth.allows(crate::serve_clients::Scope::Prompt)
            && auth.allows(crate::serve_clients::Scope::Approve))
        {
            return err_response(
                axum::http::StatusCode::FORBIDDEN,
                "terminal chats can only be opened by a connection that may prompt and approve",
            );
        }
        let rev = params.rev;
        return ws.on_upgrade(move |socket| {
            serve_local_control::pump_proxy(
                socket,
                serve_local_control::upstream_ws_url(&control_url, rev),
            )
        });
    }
    if let Some(rejection) = reject_ws_if_local_read_only(&state.store, &params.session) {
        return rejection;
    }
    let Some(handle) = state.registry.get(&params.session).await else {
        return crate::serve_agents::child_or_404(&state, &params.session, params.rev, ws).await;
    };
    let snapshot_rx = handle.snapshot_rx.clone();
    let events = handle.events.clone();
    let input_tx = handle.input_tx.clone();
    let clients = handle.ws_clients.clone();
    let going_away = Some(state.registry.signals.going_away_rx());
    let gate = crate::serve_access::InputGate::new(auth.clone(), &params.session);
    ws.on_upgrade(move |socket| async move {
        let _attached = WsClientGuard::attach(clients);
        let access = crate::serve_access::access();
        let pump = remote::pump_ws_draining(
            socket,
            snapshot_rx,
            events,
            input_tx,
            params.rev,
            going_away,
            Some(gate),
        );
        tokio::select! {
            _ = pump => {}
            _ = access.clients.until_revoked(&auth) => {}
        }
    })
}
