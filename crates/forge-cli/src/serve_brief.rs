//! `GET {base}/api/brief?since=<unix seconds>` — the "while you were away" digest.
//!
//! One compact answer over the daemon's sessions that did anything since `since`: how each one
//! ended (finished, failed with a reason, waiting on a person, cut off by a restart, still
//! working), a one-line summary, what it spent in the window, and whether its tree holds changes
//! nobody has landed. Built from the store plus the live registry; the aggregation is a pure
//! function so the buckets are testable without a daemon.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use forge_store::{BriefSessionRow, RESTART_INTERRUPT_NOTE};

use crate::serve::{err_response, json_response, DaemonState};

const MAX_SESSIONS: usize = 40;
const MAX_WINDOW_SECS: i64 = 30 * 24 * 60 * 60;
const SUMMARY_CHARS: usize = 140;
const FAILED_NOTE: &str = "⚠ Turn failed:";
/// The core's note for a turn that stopped short without an error (turn_record.rs).
const STOPPED_NOTE: &str = "⚠ Turn stopped";

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(&format!("{base}/api/brief"), get(brief))
}

/// What the registry knows about a session the daemon is driving right now.
#[derive(Debug, Clone, Default)]
pub(crate) struct Live {
    pub(crate) title: String,
    pub(crate) busy: bool,
    pub(crate) waiting: bool,
    pub(crate) failed: bool,
    pub(crate) stop_reason: Option<String>,
    pub(crate) last_activity: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Waiting,
    Failed,
    Interrupted,
    Working,
    Finished,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Unlanded {
    pub(crate) added: u32,
    pub(crate) removed: u32,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct BriefSession {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) project: String,
    pub(crate) outcome: Outcome,
    /// Failure reason, for `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// Unix seconds a waiting session has been blocked since (its last activity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) waiting_since: Option<i64>,
    pub(crate) last_activity: i64,
    pub(crate) summary: String,
    pub(crate) spend_usd: f64,
    pub(crate) worktree: bool,
    #[serde(default)]
    pub(crate) unlanded: Option<Unlanded>,
}

#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct BriefTotals {
    pub(crate) sessions: usize,
    pub(crate) finished: usize,
    pub(crate) failed: usize,
    pub(crate) waiting: usize,
    pub(crate) interrupted: usize,
    pub(crate) working: usize,
    pub(crate) changes_ready: usize,
    pub(crate) spend_usd: f64,
}

/// The newest issue drain, as one line: `Drain: 7 started, 2 running, 4 merged, 2 waiting on you, 1 blocked`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DrainBrief {
    pub(crate) id: String,
    pub(crate) line: String,
    pub(crate) status: String,
    pub(crate) queued: usize,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Brief {
    pub(crate) since: i64,
    pub(crate) now: i64,
    pub(crate) totals: BriefTotals,
    pub(crate) sessions: Vec<BriefSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) drain: Option<DrainBrief>,
}

fn first_line(text: &str, limit: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() <= limit {
        return line.to_string();
    }
    let head: String = line.chars().take(limit - 1).collect();
    format!("{}…", head.trim_end())
}

fn project_of(cwd: &str) -> String {
    std::path::Path::new(cwd.trim_end_matches('/'))
        .file_name()
        .map_or_else(|| cwd.to_string(), |n| n.to_string_lossy().into_owned())
}

fn classify(row: &BriefSessionRow, live: Option<&Live>) -> (Outcome, Option<String>) {
    if let Some(live) = live {
        if live.waiting {
            return (Outcome::Waiting, None);
        }
        if live.busy {
            return (Outcome::Working, None);
        }
    }
    let note = (row.last_role.as_deref() == Some("system"))
        .then_some(row.last_content.as_deref())
        .flatten();
    if let Some(reason) = note.and_then(|n| n.strip_prefix(FAILED_NOTE)) {
        return (Outcome::Failed, Some(first_line(reason, SUMMARY_CHARS)));
    }
    if let Some(reason) = note.and_then(|n| n.strip_prefix(STOPPED_NOTE)) {
        return (Outcome::Failed, Some(first_line(reason, SUMMARY_CHARS)));
    }
    if note.is_some_and(|n| n.starts_with(RESTART_INTERRUPT_NOTE)) {
        return (Outcome::Interrupted, None);
    }
    if let Some(live) = live.filter(|l| l.failed) {
        let reason = live
            .stop_reason
            .clone()
            .unwrap_or_else(|| "turn failed".into());
        return (Outcome::Failed, Some(reason));
    }
    (Outcome::Finished, None)
}

fn rank(outcome: Outcome) -> u8 {
    match outcome {
        Outcome::Waiting => 0,
        Outcome::Failed => 1,
        Outcome::Interrupted => 2,
        Outcome::Working => 3,
        Outcome::Finished => 4,
    }
}

pub(crate) fn build(
    rows: Vec<BriefSessionRow>,
    live: &HashMap<String, Live>,
    changes: &HashMap<String, (u32, u32)>,
    since: i64,
    now: i64,
) -> Brief {
    let mut totals = BriefTotals::default();
    let mut sessions = Vec::with_capacity(rows.len());
    for row in rows {
        let live_row = live.get(&row.id);
        let (outcome, reason) = classify(&row, live_row);
        let last_activity = live_row.map_or(row.last_activity, |l| {
            l.last_activity.max(row.last_activity)
        });
        let title = [
            row.title.as_deref(),
            live_row.map(|l| l.title.as_str()),
            row.preview.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(|t| first_line(t, 80))
        .find(|t| !t.is_empty())
        .unwrap_or_else(|| "Untitled session".into());
        let summary = match outcome {
            Outcome::Failed => reason.clone().unwrap_or_default(),
            Outcome::Interrupted => "Cut off by a daemon restart".to_string(),
            _ => row
                .last_assistant
                .as_deref()
                .map(|t| first_line(t, SUMMARY_CHARS))
                .filter(|t| !t.is_empty())
                .or_else(|| row.preview.as_deref().map(|t| first_line(t, SUMMARY_CHARS)))
                .unwrap_or_default(),
        };
        let unlanded = changes
            .get(&row.cwd)
            .filter(|(added, removed)| added + removed > 0)
            .map(|&(added, removed)| Unlanded { added, removed });
        match outcome {
            Outcome::Finished => totals.finished += 1,
            Outcome::Failed => totals.failed += 1,
            Outcome::Waiting => totals.waiting += 1,
            Outcome::Interrupted => totals.interrupted += 1,
            Outcome::Working => totals.working += 1,
        }
        totals.changes_ready += usize::from(unlanded.is_some());
        totals.spend_usd += row.spend_usd;
        sessions.push(BriefSession {
            project: project_of(&row.cwd),
            waiting_since: (outcome == Outcome::Waiting).then_some(last_activity),
            worktree: row.worktree_path.is_some(),
            id: row.id,
            title,
            cwd: row.cwd,
            outcome,
            reason,
            last_activity,
            summary,
            spend_usd: row.spend_usd,
            unlanded,
        });
    }
    totals.sessions = sessions.len();
    sessions.sort_by(|a, b| {
        rank(a.outcome)
            .cmp(&rank(b.outcome))
            .then(b.last_activity.cmp(&a.last_activity))
            .then_with(|| a.id.cmp(&b.id))
    });
    Brief {
        since,
        now,
        totals,
        sessions,
        drain: None,
    }
}

#[derive(serde::Deserialize)]
struct BriefParams {
    since: Option<i64>,
    limit: Option<usize>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

async fn brief(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<BriefParams>,
) -> Response {
    let now = now_secs();
    let since = params
        .since
        .unwrap_or(now - 8 * 3600)
        .clamp(now - MAX_WINDOW_SECS, now);
    let limit = params.limit.unwrap_or(MAX_SESSIONS).clamp(1, MAX_SESSIONS);
    let mut live: HashMap<String, Live> = HashMap::new();
    for handle in state.registry.all().await {
        let snap = handle.snapshot_rx.borrow().snapshot.clone();
        live.insert(
            handle.session_id.clone(),
            Live {
                title: handle.title(),
                busy: snap.busy,
                waiting: snap.permission_prompt.is_some() || snap.question.is_some(),
                failed: snap.last_turn_outcome.as_deref() == Some("failed"),
                stop_reason: snap.last_stop_reason.clone(),
                last_activity: handle
                    .last_activity
                    .load(std::sync::atomic::Ordering::Relaxed),
            },
        );
    }
    let store = state.store.clone();
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Brief> {
        let rows = store.brief_sessions_since(since, limit)?;
        let mut changes: HashMap<String, (u32, u32)> = HashMap::new();
        for row in &rows {
            if !changes.contains_key(&row.cwd) {
                if let Some(figures) = crate::serve::serve_session_meta::unlanded_lines(&row.cwd) {
                    changes.insert(row.cwd.clone(), figures);
                }
            }
        }
        let mut brief = build(rows, &live, &changes, since, now);
        brief.drain = crate::serve_drain::brief_line(crate::serve_drain::registry(), since);
        Ok(brief)
    })
    .await;
    match result {
        Ok(Ok(body)) => json_response(&body),
        Ok(Err(error)) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("brief unavailable: {error}"),
        ),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "brief unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, last_role: &str, last_content: &str) -> BriefSessionRow {
        BriefSessionRow {
            id: id.into(),
            title: Some(format!("title {id}")),
            preview: Some("do it".into()),
            cwd: format!("/work/{id}"),
            worktree_path: None,
            last_activity: 1000,
            done_at: None,
            spend_usd: 0.5,
            last_role: Some(last_role.into()),
            last_content: Some(last_content.into()),
            last_assistant: Some("All done.\nSecond line.".into()),
        }
    }

    fn live(f: impl FnOnce(&mut Live)) -> Live {
        let mut l = Live::default();
        f(&mut l);
        l
    }

    #[test]
    fn buckets_reasons_and_totals() {
        let rows = vec![
            row("fin", "assistant", "All done."),
            row("bad", "system", "⚠ Turn failed: provider 503\nretry later"),
            row("cut", "system", "⚠ Interrupted by a daemon restart."),
            row("wait", "assistant", "may I?"),
            row("busy", "tool", "x"),
        ];
        let mut lives = HashMap::new();
        lives.insert(
            "wait".to_string(),
            live(|l| {
                l.waiting = true;
                l.last_activity = 2000;
            }),
        );
        lives.insert("busy".to_string(), live(|l| l.busy = true));
        let mut changes = HashMap::new();
        changes.insert("/work/fin".to_string(), (3, 1));
        changes.insert("/work/bad".to_string(), (0, 0));
        let brief = build(rows, &lives, &changes, 500, 3000);

        let order: Vec<_> = brief
            .sessions
            .iter()
            .map(|s| (s.id.as_str(), s.outcome))
            .collect();
        assert_eq!(
            order,
            vec![
                ("wait", Outcome::Waiting),
                ("bad", Outcome::Failed),
                ("cut", Outcome::Interrupted),
                ("busy", Outcome::Working),
                ("fin", Outcome::Finished),
            ]
        );
        assert_eq!(brief.sessions[0].waiting_since, Some(2000));
        assert_eq!(brief.sessions[1].reason.as_deref(), Some("provider 503"));
        assert_eq!(brief.sessions[4].summary, "All done.");
        assert!(brief.sessions[4].unlanded.is_some());
        assert!(
            brief.sessions[1].unlanded.is_none(),
            "zero lines is not ready to land"
        );
        let t = &brief.totals;
        assert_eq!(
            (
                t.sessions,
                t.finished,
                t.failed,
                t.waiting,
                t.interrupted,
                t.working
            ),
            (5, 1, 1, 1, 1, 1)
        );
        assert_eq!(t.changes_ready, 1);
        assert!((t.spend_usd - 2.5).abs() < 1e-9);
    }

    #[test]
    fn a_live_failure_without_a_note_still_fails_and_summaries_truncate() {
        let mut r = row("x", "assistant", "ok");
        r.last_assistant = Some("a".repeat(300));
        let mut lives = HashMap::new();
        lives.insert(
            "x".to_string(),
            live(|l| {
                l.failed = true;
                l.stop_reason = Some("rate_limited".into());
            }),
        );
        let brief = build(vec![r], &lives, &HashMap::new(), 0, 10);
        assert_eq!(brief.sessions[0].outcome, Outcome::Failed);
        assert_eq!(brief.sessions[0].reason.as_deref(), Some("rate_limited"));

        let long = row("y", "assistant", "ok");
        let mut long = long;
        long.last_assistant = Some("b".repeat(300));
        let brief = build(vec![long], &HashMap::new(), &HashMap::new(), 0, 10);
        assert_eq!(brief.sessions[0].summary.chars().count(), SUMMARY_CHARS);
    }

    #[test]
    fn end_to_end_over_a_temp_store() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let id = store.create_session("/repo/app", "default").unwrap();
        store
            .add_message(&id, 0, forge_types::Role::User, "ship it", None)
            .unwrap();
        store
            .add_message(&id, 1, forge_types::Role::Assistant, "Shipped.", None)
            .unwrap();
        let rows = store.brief_sessions_since(0, 10).unwrap();
        let brief = build(rows, &HashMap::new(), &HashMap::new(), 0, now_secs());
        assert_eq!(brief.sessions.len(), 1);
        assert_eq!(brief.sessions[0].project, "app");
        assert_eq!(brief.sessions[0].outcome, Outcome::Finished);
        assert_eq!(brief.sessions[0].summary, "Shipped.");
    }
}
