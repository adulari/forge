//! `GET /api/sessions/{id}/history/search?q=<words>&limit=<n>`: ranked word search over a
//! session's whole history, the rows a compaction folded away included. The app's Find on
//! `/api/history?q=` matches one substring in the loaded order; this one ranks, shows the match in
//! context and says which hits the model no longer has in its context. Hits carry a `seq` the
//! app's jump-to-message (`/api/history?around=<seq>`) resolves.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use crate::serve::{err_response, json_response, DaemonState};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;
const QUERY_CHARS: usize = 200;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(
        &format!("{base}/api/sessions/{{id}}/history/search"),
        get(search),
    )
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, PartialEq, serde::Serialize)]
struct Hit {
    seq: i64,
    role: String,
    snippet: String,
    /// Folded out of the model's context by a compaction (still stored, still readable).
    folded: bool,
}

#[derive(Debug, PartialEq, serde::Serialize)]
struct SearchResponse {
    hits: Vec<Hit>,
}

fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn respond(hits: Vec<forge_store::FoldedHit>) -> SearchResponse {
    SearchResponse {
        hits: hits
            .into_iter()
            .map(|hit| Hit {
                seq: hit.seq,
                role: hit.role,
                snippet: hit.snippet,
                folded: hit.folded,
            })
            .collect(),
    }
}

async fn search(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Query(params): Query<SearchQuery>,
) -> Response {
    let query: String = params.q.trim().chars().take(QUERY_CHARS).collect();
    let limit = clamp_limit(params.limit);
    let store = state.store.clone();
    let found = tokio::task::spawn_blocking(move || {
        if store.session_cwd(&id)?.is_none() {
            return Ok(None);
        }
        store.search_session_history(&id, &query, limit).map(Some)
    })
    .await;
    match found {
        Ok(Ok(Some(hits))) => json_response(&respond(hits)),
        Ok(Ok(None)) => err_response(StatusCode::NOT_FOUND, "no such session"),
        _ => err_response(StatusCode::INTERNAL_SERVER_ERROR, "history search failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn get_json(
        store: Arc<forge_store::Store>,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let state = Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store,
            base: "/tok".into(),
            mock: true,
            default_cwd: "/tmp".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let response = routes("/tok")
            .with_state(state)
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn the_route_finds_folded_and_live_rows_and_stays_in_its_session() {
        let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
        let sid = store.create_session("/tmp", "default").unwrap();
        let other = store.create_session("/tmp", "default").unwrap();
        for (seq, text) in [
            "we settled on exponential backoff",
            "b",
            "c",
            "d",
            "e",
            "live backoff note",
        ]
        .iter()
        .enumerate()
        {
            store
                .add_message(&sid, seq as i64, forge_types::Role::User, text, None)
                .unwrap();
        }
        store
            .add_message(&other, 0, forge_types::Role::User, "other backoff", None)
            .unwrap();
        store.compact_session_store(&sid, "S", 2).unwrap();

        let (status, body) = get_json(
            store.clone(),
            &format!("/tok/api/sessions/{sid}/history/search?q=backoff"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let hits = body["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 2, "{body}");
        let folded: Vec<bool> = hits
            .iter()
            .map(|h| h["folded"].as_bool().unwrap())
            .collect();
        assert!(folded.contains(&true) && folded.contains(&false), "{body}");
        assert!(hits
            .iter()
            .all(|h| h["snippet"].as_str().unwrap().contains("[backoff]")));

        let (status, _) = get_json(store, "/tok/api/sessions/nope/history/search?q=backoff").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn the_limit_is_bounded_and_defaults() {
        assert_eq!(clamp_limit(None), DEFAULT_LIMIT);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(10_000)), MAX_LIMIT);
    }

    #[test]
    fn hits_serialize_with_the_folded_flag() {
        let body = respond(vec![forge_store::FoldedHit {
            seq: 4,
            role: "assistant".into(),
            snippet: "settled on [backoff]".into(),
            folded: true,
        }]);
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"hits": [{
                "seq": 4, "role": "assistant", "snippet": "settled on [backoff]", "folded": true
            }]})
        );
    }
}
