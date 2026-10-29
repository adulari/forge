//! `GET`/`PUT /api/keybindings`: the shared `keybindings.json` the app syncs its shortcut
//! overrides through, so the desktop, the browser and the TUI read one file.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Json, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::serve::{err_response, json_response, DaemonState};

#[derive(Deserialize)]
struct PutKeybindings {
    app: BTreeMap<String, serde_json::Value>,
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(
        &format!("{base}/api/keybindings"),
        get(get_keybindings).put(put_keybindings),
    )
}

async fn get_keybindings(State(_): State<Arc<DaemonState>>) -> Response {
    let file = forge_config::keybindings_path()
        .map(|path| forge_config::read_keybindings_file(&path))
        .unwrap_or_default();
    json_response(&file)
}

async fn put_keybindings(
    State(_): State<Arc<DaemonState>>,
    Json(body): Json<PutKeybindings>,
) -> Response {
    let Some(path) = forge_config::keybindings_path() else {
        return err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "no config directory",
        );
    };
    match forge_config::write_app_overrides(&path, body.app) {
        Ok(file) => json_response(&file),
        Err(error) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &error.to_string(),
        ),
    }
}
