//! Socket routes that end the moment the client behind them is revoked.

use crate::serve_access::access;
use crate::serve_clients::ClientAuth;

/// Upgrades a socket and drops it the moment `auth`'s client is revoked. Every socket route
/// must go through this so a revoked device cannot keep a live connection.
pub(crate) fn upgrade_until_revoked<F, Fut>(
    ws: axum::extract::WebSocketUpgrade,
    auth: ClientAuth,
    serve: F,
) -> axum::response::Response
where
    F: FnOnce(axum::extract::ws::WebSocket) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    ws.on_upgrade(move |socket| run_until_revoked(auth, serve(socket)))
}

/// Runs `work` until it finishes or `auth`'s client is revoked, whichever is first.
pub(crate) async fn run_until_revoked(
    auth: ClientAuth,
    work: impl std::future::Future<Output = ()>,
) {
    let access = access();
    tokio::select! {
        _ = work => {}
        _ = access.clients.until_revoked(&auth) => {}
    }
}
