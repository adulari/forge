//! The board's HTTP half: the daemon's read routes (fleet, past, git status, history) and its
//! action routes (interrupt, archive, mode, create/resume). Every route is token-scoped exactly
//! like `forge attach`'s — `/<token>/api/…`, a wrong token is a 404, never a 401.
//!
//! Reads that fail are reported once, as a single toast plus a connectivity change, rather than
//! per-attempt: the board polls, and a daemon that is down would otherwise paint the screen with
//! the same error every fifteen seconds.

use anyhow::{bail, Context, Result};
use forge_tui::board::{BoardEvent, ConnState, FleetRow, GitInfo, HistoryRow, PastRow, ToastLevel};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;

use super::{Ev, HISTORY_LIMIT, PAST_LIMIT};

/// A burst of `fleet_changed` frames (one session streaming is enough to produce several) is one
/// refetch.
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);
/// The fleet is re-read this often even with no invalidation, so a dropped socket can't freeze the
/// board on stale rows.
const FALLBACK_REFRESH: std::time::Duration = std::time::Duration::from_secs(15);
/// Toasts are one line on a card-sized surface; a wall of HTML or a stack trace is useless there.
const TOAST_MAX_CHARS: usize = 110;

pub(crate) fn api_url(base: &str, token: &str, path: &str) -> String {
    format!(
        "{}/{}/{}",
        base.trim_end_matches('/'),
        token,
        path.trim_start_matches('/')
    )
}

/// The same base URL over the WebSocket scheme. `--url` is the only place a non-http scheme can
/// enter, so this is where it is rejected.
pub(crate) fn ws_url(base: &str, token: &str, path: &str) -> Result<String> {
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        bail!("--url must start with http:// or https:// (got {base})")
    };
    Ok(api_url(&ws_base, token, path))
}

pub(crate) async fn fetch_fleet(
    http: &reqwest::Client,
    base: &str,
    token: &str,
) -> Result<Vec<FleetRow>> {
    get_json(http, base, &api_url(base, token, "api/sessions")).await
}

pub(crate) async fn fetch_past(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    limit: usize,
) -> Result<Vec<PastRow>> {
    let url = api_url(base, token, &format!("api/sessions/past?limit={limit}"));
    get_json(http, base, &url).await
}

async fn get_json<T: DeserializeOwned>(http: &reqwest::Client, base: &str, url: &str) -> Result<T> {
    let resp = http.get(url).send().await.map_err(|e| {
        anyhow::anyhow!(
            "could not reach the forge serve daemon at {base} — is it running? \
             (start it with `forge serve --local`)  [{e}]"
        )
    })?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("daemon rejected the token (404) — wrong --token, or the daemon rotated it");
    }
    if !resp.status().is_success() {
        let status = resp.status();
        bail!("daemon returned {status}: {}", body_message(resp).await);
    }
    resp.json::<T>()
        .await
        .with_context(|| format!("daemon response for {url} was not the expected JSON"))
}

/// The human half of a daemon error response (`{"error": "…"}`), falling back to whatever the body
/// actually was.
async fn body_message(resp: reqwest::Response) -> String {
    error_message(&resp.text().await.unwrap_or_default())
}

pub(crate) fn error_message(body: &str) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string));
    let text = parsed.unwrap_or_else(|| body.trim().to_string());
    let one_line = text.split('\n').next().unwrap_or_default().trim();
    if one_line.is_empty() {
        return "the daemon gave no reason".to_string();
    }
    truncate(one_line, TOAST_MAX_CHARS)
}

pub(crate) fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

// ---------------------------------------------------------------------------
// The fleet poller
// ---------------------------------------------------------------------------

/// Owns every `GET /api/sessions` the board makes: on request (the fleet socket saw a change, or
/// the user pressed R), and on a fallback timer. Requests are debounced and the timer is reset
/// after each fetch, so a busy fleet costs one refetch per 300 ms rather than one per frame.
pub(crate) async fn fleet_refresher(
    http: reqwest::Client,
    base: String,
    token: String,
    mut requests: mpsc::UnboundedReceiver<()>,
    ev: mpsc::UnboundedSender<Ev>,
) {
    let mut fallback = tokio::time::interval(FALLBACK_REFRESH);
    fallback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    fallback.tick().await; // the immediate first tick: the caller already has a fleet
    let mut offline = false;
    loop {
        tokio::select! {
            request = requests.recv() => {
                if request.is_none() {
                    return;
                }
                if !debounce(&mut requests).await {
                    return;
                }
            }
            _ = fallback.tick() => {}
        }
        refresh_once(&http, &base, &token, &ev, &mut offline).await;
        fallback.reset();
    }
}

/// Swallow further requests for [`DEBOUNCE`]. `false` means the channel closed — shut down.
async fn debounce(requests: &mut mpsc::UnboundedReceiver<()>) -> bool {
    let deadline = tokio::time::Instant::now() + DEBOUNCE;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => return true,
            request = requests.recv() => {
                if request.is_none() {
                    return false;
                }
            }
        }
    }
}

async fn refresh_once(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    ev: &mpsc::UnboundedSender<Ev>,
    offline: &mut bool,
) {
    let (fleet, past) = tokio::join!(
        fetch_fleet(http, base, token),
        fetch_past(http, base, token, PAST_LIMIT)
    );
    match fleet {
        Ok(rows) => {
            if *offline {
                *offline = false;
                // Only un-report an outage this poller itself reported; the fleet socket owns
                // Live/Reconnecting the rest of the time.
                let _ = ev.send(Ev::Board(BoardEvent::Connection(ConnState::Live)));
            }
            let _ = ev.send(Ev::Board(BoardEvent::Fleet(rows)));
        }
        Err(e) => {
            if !*offline {
                *offline = true;
                let reason = truncate(&e.to_string(), TOAST_MAX_CHARS);
                let _ = ev.send(Ev::Board(BoardEvent::Connection(ConnState::Offline(
                    reason.clone(),
                ))));
                let _ = ev.send(Ev::Board(BoardEvent::Toast(ToastLevel::Error, reason)));
            }
            return;
        }
    }
    if let Ok(rows) = past {
        let _ = ev.send(Ev::Board(BoardEvent::Past(rows)));
    }
}

// ---------------------------------------------------------------------------
// The detail pane
// ---------------------------------------------------------------------------

/// Everything the detail pane needs that isn't in the live snapshot, fetched together so opening a
/// card is one round trip's worth of latency, and reported as ONE toast when it fails.
pub(crate) async fn fetch_detail(
    http: reqwest::Client,
    base: String,
    token: String,
    id: String,
    ev: mpsc::UnboundedSender<Ev>,
) {
    let (git_url, history_url) = (git_url(&base, &token, &id), history_url(&base, &token, &id));
    let (git, history) = tokio::join!(
        get_json::<GitInfo>(&http, &base, &git_url),
        get_json::<Vec<HistoryRow>>(&http, &base, &history_url)
    );

    // A session outside a git repository is normal (a scratch directory, a fresh project), and
    // the daemon says so with an error status — the Changes tab reads "no git info yet", which is
    // the whole message. Only a missing history is worth a toast.
    if let Ok(info) = git {
        let _ = ev.send(Ev::Board(BoardEvent::Git(id.clone(), info)));
    }
    match history {
        Ok(rows) => {
            let _ = ev.send(Ev::Board(BoardEvent::History(id.clone(), rows)));
        }
        Err(_) => {
            let _ = ev.send(Ev::Board(BoardEvent::Toast(
                ToastLevel::Error,
                format!("history unavailable for {}", super::short_id(&id)),
            )));
        }
    }
}

/// The open pane's working tree, re-read on a timer (a running session commits and edits while the
/// pane is open). Silent on failure — [`fetch_detail`] already said so when the pane opened.
pub(crate) async fn fetch_git(
    http: reqwest::Client,
    base: String,
    token: String,
    id: String,
    ev: mpsc::UnboundedSender<Ev>,
) {
    if let Ok(info) = get_json::<GitInfo>(&http, &base, &git_url(&base, &token, &id)).await {
        let _ = ev.send(Ev::Board(BoardEvent::Git(id, info)));
    }
}

fn git_url(base: &str, token: &str, id: &str) -> String {
    api_url(base, token, &format!("api/git/status?session={id}"))
}

fn history_url(base: &str, token: &str, id: &str) -> String {
    api_url(
        base,
        token,
        &format!("api/history?session={id}&limit={HISTORY_LIMIT}&include_tools=1"),
    )
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// `POST` one action and report it. `ok` is what the toast says on success; the daemon's own
/// `{"error": …}` text is what it says on failure, because that string is the one that explains
/// why (a session that is shutting down, a mode the daemon doesn't know, a missing worktree).
pub(crate) async fn post_action(
    http: reqwest::Client,
    url: String,
    body: serde_json::Value,
    ok: String,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    match post(&http, &url, &body).await {
        Ok(_) => {
            let _ = ev.send(Ev::Board(BoardEvent::Toast(ToastLevel::Ok, ok)));
        }
        Err(e) => {
            let _ = ev.send(Ev::Board(BoardEvent::Toast(
                ToastLevel::Error,
                truncate(&e.to_string(), TOAST_MAX_CHARS),
            )));
        }
    }
    let _ = refresh.send(());
}

/// `POST /api/sessions` for a brand-new session. The prompt cannot ride along — the daemon takes
/// prompts over the session's own socket — so the created id comes back to the render loop, which
/// holds the text until that socket opens.
pub(crate) async fn create_session(
    http: reqwest::Client,
    url: String,
    body: serde_json::Value,
    prompt: String,
    ev: mpsc::UnboundedSender<Ev>,
    refresh: mpsc::UnboundedSender<()>,
) {
    match post(&http, &url, &body).await {
        Ok(value) => match value["id"].as_str() {
            Some(id) => {
                let _ = ev.send(Ev::Board(BoardEvent::Toast(
                    ToastLevel::Ok,
                    format!("session {} started", super::short_id(id)),
                )));
                let _ = ev.send(Ev::Created(id.to_string(), prompt));
            }
            None => {
                let _ = ev.send(Ev::Board(BoardEvent::Toast(
                    ToastLevel::Error,
                    "the daemon created a session but did not say which".to_string(),
                )));
            }
        },
        Err(e) => {
            let _ = ev.send(Ev::Board(BoardEvent::Toast(
                ToastLevel::Error,
                truncate(&e.to_string(), TOAST_MAX_CHARS),
            )));
        }
    }
    let _ = refresh.send(());
}

async fn post(
    http: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let resp = http
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("the daemon did not answer: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("the daemon does not know that session (or the token is wrong)");
    }
    if !resp.status().is_success() {
        bail!("{}", body_message(resp).await);
    }
    Ok(resp.json::<serde_json::Value>().await.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_is_scoped_by_the_daemon_token() {
        assert_eq!(
            api_url("http://127.0.0.1:7420", "tok", "api/sessions"),
            "http://127.0.0.1:7420/tok/api/sessions"
        );
        // A trailing slash on --url must not double up into `//tok`, which the daemon 404s.
        assert_eq!(
            api_url("http://host:9/", "tok", "/api/sessions/past?limit=30"),
            "http://host:9/tok/api/sessions/past?limit=30"
        );
    }

    #[test]
    fn websocket_routes_keep_the_scheme_and_the_token() {
        assert_eq!(
            ws_url("http://127.0.0.1:7420", "tok", "ws/fleet").unwrap(),
            "ws://127.0.0.1:7420/tok/ws/fleet"
        );
        assert_eq!(
            ws_url("https://box.example", "tok", "ws?session=a&rev=3").unwrap(),
            "wss://box.example/tok/ws?session=a&rev=3"
        );
        assert!(ws_url("ftp://nope", "tok", "ws/fleet").is_err());
    }

    #[test]
    fn detail_routes_ask_for_tool_rows_and_a_bounded_page() {
        assert_eq!(
            history_url("http://h:1", "tok", "s9"),
            format!("http://h:1/tok/api/history?session=s9&limit={HISTORY_LIMIT}&include_tools=1")
        );
        assert_eq!(
            git_url("http://h:1", "tok", "s9"),
            "http://h:1/tok/api/git/status?session=s9"
        );
    }

    #[test]
    fn a_failure_reads_as_the_daemons_own_reason() {
        assert_eq!(
            error_message(r#"{"error":"session driver is no longer accepting input"}"#),
            "session driver is no longer accepting input"
        );
        // Not the daemon's shape (a proxy's HTML, say): show the first line, not the whole page.
        assert_eq!(error_message("<html>\n<body>502</body>"), "<html>");
        assert_eq!(error_message("   "), "the daemon gave no reason");
    }

    #[test]
    fn a_long_reason_is_cut_to_one_toast_line() {
        let long = "x".repeat(400);
        let cut = truncate(&long, TOAST_MAX_CHARS);
        assert_eq!(cut.chars().count(), TOAST_MAX_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(truncate("short", TOAST_MAX_CHARS), "short");
        // Cutting must land on a character, never a byte.
        assert_eq!(truncate("ünïcødé", 4), "ünï…");
    }
}
