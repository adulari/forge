//! `GET`/`PUT`/`DELETE /api/crews`: saved crews, the named rosters a session or an `@crew:name`
//! mention delegates to (see `forge_config::agents::crews`). Reads merge the user's crews with the
//! project's; writes and deletes touch the user's own directory only, so a project's checked-in
//! crews are shown but never edited from here.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Json, Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use forge_config::agents::crews::{self, Crew, CrewMember};
use serde::{Deserialize, Serialize};

use crate::serve::{err_response, json_response, DaemonState};

#[derive(Deserialize)]
struct CwdQuery {
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct DeleteQuery {
    name: String,
}

#[derive(Deserialize)]
struct PutCrew {
    name: String,
    #[serde(default)]
    description: String,
    members: Vec<CrewMember>,
    /// Refuse (409) when a crew of this name already exists in any layer, so "New crew" can
    /// never overwrite one or save a name a nearer project crew would shadow.
    #[serde(default)]
    create_only: bool,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Serialize)]
struct CrewRow {
    name: String,
    description: String,
    members: Vec<CrewMember>,
    /// `"user"` (the user's own crew directories) or `"project"` (a checked-in `.forge/crews`).
    scope: &'static str,
    /// `false` for a project crew: shown, not editable here.
    editable: bool,
}

#[derive(Serialize)]
struct CrewList {
    crews: Vec<CrewRow>,
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(
        &format!("{base}/api/crews"),
        get(list_crews).put(put_crew).delete(delete_crew),
    )
}

fn row(crew: Crew, user_dirs: &[PathBuf]) -> CrewRow {
    let user = crew
        .path
        .as_ref()
        .and_then(|p| p.parent())
        .is_some_and(|dir| user_dirs.iter().any(|u| u.as_path() == dir));
    CrewRow {
        name: crew.name,
        description: crew.description,
        members: crew.members,
        scope: if user { "user" } else { "project" },
        editable: user,
    }
}

async fn list_crews(State(_): State<Arc<DaemonState>>, Query(q): Query<CwdQuery>) -> Response {
    let root = q
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let user_dirs = crews::user_crews_dirs();
    let crews = crews::load_crews(&root)
        .into_iter()
        .map(|c| row(c, &user_dirs))
        .collect();
    json_response(&CrewList { crews })
}

async fn put_crew(State(_): State<Arc<DaemonState>>, Json(body): Json<PutCrew>) -> Response {
    let Some(dir) = crews::user_crews_dir() else {
        return err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "no config directory",
        );
    };
    if body.create_only {
        let root = body
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if crews::load_crews(&root).iter().any(|c| c.name == body.name) {
            return err_response(
                axum::http::StatusCode::CONFLICT,
                "a crew with that name already exists",
            );
        }
    }
    let crew = Crew {
        name: body.name,
        description: body.description.trim().to_string(),
        members: body.members,
        path: None,
    };
    match crews::save_crew_in(&dir, &crew) {
        Ok(path) => json_response(&serde_json::json!({ "ok": true, "path": path })),
        Err(problem) => err_response(axum::http::StatusCode::BAD_REQUEST, &problem),
    }
}

async fn delete_crew(State(_): State<Arc<DaemonState>>, Query(q): Query<DeleteQuery>) -> Response {
    let Some(dir) = crews::user_crews_dir() else {
        return err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "no config directory",
        );
    };
    // A crew may sit in more than one user directory; deleting it removes every user copy.
    let mut removed = false;
    for dir in crews::user_crews_dirs().iter().chain(std::iter::once(&dir)) {
        match crews::delete_crew_in(dir, &q.name) {
            Ok(found) => removed |= found,
            Err(e) => return err_response(axum::http::StatusCode::BAD_REQUEST, &e),
        }
    }
    if removed {
        json_response(&serde_json::json!({ "ok": true }))
    } else {
        err_response(axum::http::StatusCode::NOT_FOUND, "no such crew")
    }
}
