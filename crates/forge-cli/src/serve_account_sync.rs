//! `GET`/`PUT /api/anywhere/sync` and `POST /api/anywhere/sync/now`: which parts of this machine's
//! setup follow the Forge Anywhere account, and what the last cycles did.
//!
//! The status is bridge-readable. Switching `keys` on or off is not: the Anywhere bridge listener
//! stamps every request it forwards, and a stamped request may never touch credential sync, here
//! or through `PUT /api/config`.

use std::sync::Arc;

use axum::extract::{Json, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::anywhere::account_sync::{self, Category};
use crate::serve::{err_response, err_response_coded, json_response, DaemonState};

const BRIDGE_HEADER: &str = "x-forge-anywhere-bridge";

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/anywhere/sync"),
            get(status_page).put(update),
        )
        .route(&format!("{base}/api/anywhere/sync/now"), post(sync_now))
}

/// Layer for the loopback listener the Anywhere connector forwards bridge requests through.
pub(crate) async fn mark_bridge_origin(mut request: Request, next: Next) -> Response {
    request
        .headers_mut()
        .insert(BRIDGE_HEADER, HeaderValue::from_static("1"));
    next.run(request).await
}

pub(crate) fn from_bridge(headers: &HeaderMap) -> bool {
    headers.contains_key(BRIDGE_HEADER)
}

pub(crate) fn keys_toggle_refused() -> Response {
    err_response_coded(
        StatusCode::FORBIDDEN,
        "direct_only",
        "Keys & tokens can only be switched on the machine itself",
    )
}

#[derive(Deserialize)]
struct UpdateBody {
    category: String,
    enabled: bool,
}

async fn status_page(State(state): State<Arc<DaemonState>>) -> Response {
    snapshot(state.mock).await
}

async fn snapshot(mock: bool) -> Response {
    if mock {
        return json_response(&mock_status());
    }
    match tokio::task::spawn_blocking(|| account_sync::status_snapshot(&crate::open_store()?)).await
    {
        Ok(Ok(status)) => json_response(&status),
        Ok(Err(error)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error:#}")),
        Err(_) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read sync status",
        ),
    }
}

async fn update(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> Response {
    let Some(category) = Category::parse(&body.category) else {
        return err_response(StatusCode::BAD_REQUEST, "unknown sync category");
    };
    if category == Category::Keys && from_bridge(&headers) {
        return keys_toggle_refused();
    }
    if !state.mock {
        if !crate::anywhere::signed_in() {
            return err_response_coded(
                StatusCode::CONFLICT,
                "not_signed_in",
                account_sync::not_signed_in_message(),
            );
        }
        let result = tokio::task::spawn_blocking(move || {
            let _lock = crate::serve::config_mutation_lock()
                .lock()
                .map_err(|_| "config lock poisoned".to_string())?;
            forge_config::account_sync::set_account_sync_flag(category.id(), body.enabled)
                .map_err(|e| e.to_string())
        })
        .await;
        match result {
            Ok(Ok(())) => account_sync::request_sync_now(),
            Ok(Err(error)) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, &error),
            Err(_) => {
                return err_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "could not update sync settings",
                )
            }
        }
    }
    snapshot(state.mock).await
}

async fn sync_now() -> Response {
    account_sync::request_sync_now();
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

fn mock_status() -> serde_json::Value {
    serde_json::json!({
        "signed_in": true,
        "anywhere_enabled": true,
        "last_sync_at": 1_793_390_000_000_u64,
        "last_error": null,
        "categories": [
            { "id": "keys", "label": "Keys & tokens", "enabled": false, "items": 0, "pending": 0,
              "last_sync_at": null, "remote_only": 3, "conflicts": [], "skipped": [],
              "toggle_requires_direct": true },
            { "id": "skills", "label": "Skills & agents", "enabled": true, "items": 6, "pending": 1,
              "last_sync_at": 1_793_390_000_000_u64, "remote_only": 0, "conflicts": [], "skipped": [],
              "toggle_requires_direct": false },
            { "id": "settings", "label": "Settings", "enabled": true, "items": 9, "pending": 0,
              "last_sync_at": 1_793_390_000_000_u64, "remote_only": 0,
              "conflicts": [{ "item": "setting: tui.theme", "detail": "changed here and on another device; kept the other device's" }],
              "skipped": [], "toggle_requires_direct": false },
            { "id": "mcp", "label": "MCP servers", "enabled": true, "items": 2, "pending": 0,
              "last_sync_at": 1_793_390_000_000_u64, "remote_only": 1, "conflicts": [],
              "skipped": [{ "item": "mcp: github", "reason": "needs `npx`, not installed here" }],
              "toggle_requires_direct": false },
            { "id": "instructions", "label": "Instructions", "enabled": false, "items": 0, "pending": 0,
              "last_sync_at": null, "remote_only": 1, "conflicts": [], "skipped": [],
              "toggle_requires_direct": false }
        ],
        "sign_ins": [{ "provider": "codex", "label": "Codex (ChatGPT)", "command": "forge auth codex-oauth" }],
        "missing_tools": [{ "tool": "claude", "label": "Claude Code", "hint": "npm install -g @anthropic-ai/claude-code" }]
    })
}
