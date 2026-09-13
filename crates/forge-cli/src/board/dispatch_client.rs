//! The host half of plan & dispatch: reading `GET /api/dispatches`, starting a dispatch, the
//! approve/revise/cancel routes, and merging or discarding worker worktrees — including merging
//! every finished worker of a dispatch in order, stopping at the first conflict.
//!
//! A POST failure keeps its status and JSON body rather than flattening to a string, because a
//! merge conflict (409 with `conflicts`) must read differently from any other refusal.

use anyhow::{bail, Result};
use forge_tui::board::{project_name, BoardEvent, DispatchInfo, ToastLevel};
use tokio::sync::mpsc;

use super::client::{api_url, error_message, truncate};
use super::{short_id, Ev};

/// Dispatches asked for on every refresh — recent ones are what a board shows.
pub(crate) const DISPATCH_LIMIT: usize = 30;
const TOAST_MAX_CHARS: usize = 110;
const TITLE_IN_TOAST: usize = 20;

/// `GET /api/dispatches`. A daemon that predates plan & dispatch answers 404, which is simply no
/// dispatches — the rest of the board works exactly as before.
pub(crate) async fn fetch_dispatches(
    http: &reqwest::Client,
    base: &str,
    token: &str,
) -> Result<Vec<DispatchInfo>> {
    let url = api_url(
        base,
        token,
        &format!("api/dispatches?limit={DISPATCH_LIMIT}"),
    );
    let resp = http.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        bail!(
            "daemon returned {status}: {}",
            error_message(&resp.text().await.unwrap_or_default())
        );
    }
    Ok(resp.json::<Vec<DispatchInfo>>().await?)
}

/// A refused or failed POST.
#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) status: Option<u16>,
    pub(crate) body: serde_json::Value,
    pub(crate) message: String,
}

pub(crate) async fn post_json(
    http: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, Failure> {
    let resp = http
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| Failure {
            status: None,
            body: serde_json::Value::Null,
            message: format!("the daemon did not answer: {e}"),
        })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let json = serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default();
    if status.is_success() {
        return Ok(json);
    }
    let message = if status == reqwest::StatusCode::NOT_FOUND && json["error"].is_null() {
        "the daemon does not know that (is it older than plan & dispatch?)".to_string()
    } else {
        error_message(&text)
    };
    Err(Failure {
        status: Some(status.as_u16()),
        body: json,
        message,
    })
}

fn toast(ev: &mpsc::UnboundedSender<Ev>, level: ToastLevel, text: String) {
    let _ = ev.send(Ev::Board(BoardEvent::Toast(
        level,
        truncate(&text, TOAST_MAX_CHARS),
    )));
}

/// `POST /api/dispatch`'s body. Only declared fields: the route denies unknown ones.
pub(crate) fn start_body(
    cwd: &str,
    prompt: &str,
    worktree: bool,
    mode: &str,
    max_running: usize,
    max_items: usize,
) -> serde_json::Value {
    serde_json::json!({
        "cwd": cwd,
        "prompt": prompt,
        "worktree": worktree,
        "mode": mode,
        "max_running": max_running,
        "max_items": max_items,
    })
}

/// Start a dispatch and tell the board which coordinator to select once it appears.
pub(crate) async fn start_dispatch(
    http: reqwest::Client,
    url: String,
    body: serde_json::Value,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    match post_json(&http, &url, &body).await {
        Ok(value) => match (
            value["dispatch_id"].as_str(),
            value["coordinator_session_id"].as_str(),
        ) {
            (Some(dispatch_id), Some(coordinator)) => {
                toast(
                    &ev,
                    ToastLevel::Ok,
                    "dispatch started — planning the split".into(),
                );
                let _ = ev.send(Ev::Board(BoardEvent::DispatchStarted {
                    dispatch_id: dispatch_id.to_string(),
                    coordinator_session_id: coordinator.to_string(),
                }));
            }
            _ => toast(
                &ev,
                ToastLevel::Error,
                "the daemon started a dispatch but did not say which".into(),
            ),
        },
        Err(f) => toast(&ev, ToastLevel::Error, f.message),
    }
    let _ = refresh.send(());
}

/// Approve / revise / cancel: one POST, one toast, one refetch.
pub(crate) async fn dispatch_post(
    http: reqwest::Client,
    url: String,
    body: serde_json::Value,
    ok: String,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    match post_json(&http, &url, &body).await {
        Ok(_) => toast(&ev, ToastLevel::Ok, ok),
        Err(f) => toast(&ev, ToastLevel::Error, f.message),
    }
    let _ = refresh.send(());
}

/// `merge conflicts: 3 files in wt-9f2` from the daemon's 409 body.
pub(crate) fn conflict_words(body: &serde_json::Value) -> Option<String> {
    let n = body.get("conflicts")?.as_array()?.len();
    let worktree = body["worktree"]
        .as_str()
        .map(project_name)
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| "its worktree".into());
    Some(format!(
        "merge conflicts: {n} file{} in {worktree}",
        if n == 1 { "" } else { "s" }
    ))
}

fn merge_failure(f: &Failure) -> String {
    conflict_words(&f.body)
        .filter(|_| f.status == Some(409))
        .unwrap_or_else(|| f.message.clone())
}

fn session_url(base: &str, token: &str, id: &str, verb: &str) -> String {
    api_url(base, token, &format!("api/sessions/{id}/{verb}"))
}

pub(crate) async fn merge(
    http: reqwest::Client,
    base: String,
    token: String,
    id: String,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    let url = session_url(&base, &token, &id, "merge");
    match post_json(&http, &url, &serde_json::json!({})).await {
        Ok(_) => toast(&ev, ToastLevel::Ok, format!("merged {}", short_id(&id))),
        Err(f) => toast(&ev, ToastLevel::Error, merge_failure(&f)),
    }
    let _ = refresh.send(());
}

pub(crate) async fn discard(
    http: reqwest::Client,
    base: String,
    token: String,
    id: String,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    let url = session_url(&base, &token, &id, "discard");
    match post_json(&http, &url, &serde_json::json!({})).await {
        Ok(_) => toast(&ev, ToastLevel::Ok, format!("discarded {}", short_id(&id))),
        Err(f) => toast(&ev, ToastLevel::Error, f.message),
    }
    let _ = refresh.send(());
}

/// Merge every `succeeded` worker of a dispatch, in index order, one at a time. The dispatch is
/// re-read first (the board's copy may be a refresh old), and the run stops at the first conflict
/// or error — later merges could depend on the one that failed — naming the item that stopped it.
pub(crate) async fn merge_finished(
    http: reqwest::Client,
    base: String,
    token: String,
    dispatch_id: String,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    let dispatch = match fetch_dispatch(&http, &base, &token, &dispatch_id).await {
        Ok(d) => d,
        Err(e) => {
            toast(&ev, ToastLevel::Error, e.to_string());
            let _ = refresh.send(());
            return;
        }
    };
    let mut ready: Vec<_> = dispatch
        .items
        .iter()
        .filter(|i| i.status == "succeeded")
        .filter_map(|i| i.session_id.clone().map(|s| (i.index, i.title.clone(), s)))
        .collect();
    ready.sort_by_key(|(index, ..)| *index);
    if ready.is_empty() {
        toast(&ev, ToastLevel::Info, "no finished session to merge".into());
        let _ = refresh.send(());
        return;
    }
    let mut merged = 0usize;
    for (index, title, session) in ready {
        let url = session_url(&base, &token, &session, "merge");
        match post_json(&http, &url, &serde_json::json!({})).await {
            Ok(_) => {
                merged += 1;
                let _ = refresh.send(());
            }
            Err(f) => {
                let title = truncate(&title, TITLE_IN_TOAST);
                let done = if merged > 0 {
                    format!(" ({merged} merged first)")
                } else {
                    String::new()
                };
                toast(
                    &ev,
                    ToastLevel::Error,
                    format!("stopped at {index} {title}: {}{done}", merge_failure(&f)),
                );
                let _ = refresh.send(());
                return;
            }
        }
    }
    toast(
        &ev,
        ToastLevel::Ok,
        format!(
            "merged {merged} session{}",
            if merged == 1 { "" } else { "s" }
        ),
    );
    let _ = refresh.send(());
}

async fn fetch_dispatch(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    id: &str,
) -> Result<DispatchInfo> {
    let url = api_url(base, token, &format!("api/dispatches/{id}"));
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("the daemon did not answer: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("that dispatch is gone");
    }
    if !resp.status().is_success() {
        bail!("{}", error_message(&resp.text().await.unwrap_or_default()));
    }
    Ok(resp.json::<DispatchInfo>().await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_body_carries_only_the_declared_fields() {
        let body = start_body("/repo", "split the parser work", true, "accept-edits", 4, 8);
        let object = body.as_object().unwrap();
        assert_eq!(object.len(), 6);
        assert_eq!(body["mode"], "accept-edits");
        assert_eq!(body["max_items"], 8);
    }

    #[test]
    fn a_conflict_names_the_file_count_and_the_worktree() {
        let body = serde_json::json!({
            "error": "merge conflicts",
            "conflicts": ["a.rs", "b.rs"],
            "worktree": "/home/me/.forge/worktrees/wt-9f2",
        });
        assert_eq!(
            conflict_words(&body).unwrap(),
            "merge conflicts: 2 files in wt-9f2"
        );
        assert!(conflict_words(&serde_json::json!({"error": "nope"})).is_none());
    }
}
