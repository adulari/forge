//! PR Watch: the daemon walks away from a pull request for you.
//!
//! A watch is armed per session (`POST /api/pr/watch`) in one of two modes. `fix` polls the
//! branch's PR through `gh`, and when a check fails, changes are requested or a new comment
//! lands, it enqueues one fix turn in the session (once per distinct issue key, never twice for
//! the same state). `merge` additionally merges when everything is green, then marks the session
//! done. The session runs in the Auto permission profile while watched, so a plain `git push` is
//! pre-authorized and a force push is refused. Anything the watcher cannot resolve is recorded as
//! `blocked_reason` instead of spending more turns.
//!
//! Routes (token-scoped like the rest of `/api/pr/*`):
//! - `GET  /api/pr/watch?session=`   the session's watch, or `null`
//! - `GET  /api/pr/watches`          every armed watch (Fleet rows, Schedules list)
//! - `POST /api/pr/watch`            `{session, mode: off|fix|merge, method?: squash|merge|rebase|auto}`
//! - `GET  /api/pr/queue`            the merge queue per repository (docs/features/merge-queue.md)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Json, Query, State};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_git::session_repo_root;
use crate::serve_pr::{
    merge_in, parse_summary, Gh, GhError, MergeRequest, PrSummary, SUMMARY_FIELDS,
};

const TICK: Duration = Duration::from_secs(2);
const POLL_SECS: i64 = 30;
const MAX_FIX_TURNS: u32 = 4;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Mode {
    Off,
    Fix,
    Merge,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Phase {
    Watching,
    Merged,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Watch {
    pub(crate) session_id: String,
    pub(crate) mode: Mode,
    /// `squash` | `merge` | `rebase` | `auto` (enable GitHub auto-merge with squash).
    pub(crate) method: String,
    pub(crate) state: Phase,
    pub(crate) blocked_reason: Option<String>,
    pub(crate) pr_number: Option<u64>,
    pub(crate) pr_url: Option<String>,
    pub(crate) head_sha: Option<String>,
    last_key: Option<String>,
    seen_comments: Vec<String>,
    pub(crate) fix_turns: u32,
    auto_armed: bool,
    pub(crate) last_checked: Option<i64>,
    pub(crate) last_action: Option<String>,
    pub(crate) created_at: i64,
    /// The transition already announced by push (`merged` / `blocked` / `green`), kept so a
    /// daemon restart never pushes the same one twice.
    #[serde(default)]
    pushed: Option<String>,
    /// Merge queue standing (merge mode only); see `serve_merge_queue`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) queue: Option<crate::serve_merge_queue::QueueInfo>,
    /// Why the merge queue is not letting this PR merge yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hold: Option<String>,
    /// The PR's base branch as of the last poll.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) base_branch: Option<String>,
}

impl Watch {
    pub(crate) fn new(session_id: &str, mode: Mode, method: &str, now: i64) -> Self {
        Self {
            session_id: session_id.to_string(),
            mode,
            method: method.to_string(),
            state: Phase::Watching,
            blocked_reason: None,
            pr_number: None,
            pr_url: None,
            head_sha: None,
            last_key: None,
            seen_comments: Vec::new(),
            fix_turns: 0,
            auto_armed: false,
            last_checked: None,
            last_action: None,
            created_at: now,
            pushed: None,
            queue: None,
            hold: None,
            base_branch: None,
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.mode != Mode::Off && self.state == Phase::Watching
    }
}

pub(crate) struct Registry {
    path: Option<PathBuf>,
    watches: Mutex<HashMap<String, Watch>>,
}

impl Registry {
    fn new(path: Option<PathBuf>) -> Self {
        let watches = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<Watch>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|w| (w.session_id.clone(), w))
            .collect();
        Self {
            path,
            watches: Mutex::new(watches),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Watch>> {
        self.watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn persist(&self, watches: &HashMap<String, Watch>) {
        let Some(path) = &self.path else { return };
        let mut rows: Vec<&Watch> = watches.values().collect();
        rows.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.session_id.cmp(&b.session_id))
        });
        let Ok(bytes) = serde_json::to_vec_pretty(&rows) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }

    pub(crate) fn get(&self, session: &str) -> Option<Watch> {
        self.lock().get(session).cloned()
    }

    pub(crate) fn all(&self) -> Vec<Watch> {
        let mut rows: Vec<Watch> = self.lock().values().cloned().collect();
        rows.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.session_id.cmp(&b.session_id))
        });
        rows
    }

    fn update(&self, session: &str, change: impl FnOnce(&mut Watch)) {
        let mut guard = self.lock();
        if let Some(watch) = guard.get_mut(session) {
            change(watch);
        }
        self.persist(&guard);
    }

    fn arm(&self, session: &str, mode: Mode, method: &str, now: i64) -> Option<Watch> {
        let mut guard = self.lock();
        let result = if mode == Mode::Off {
            guard.remove(session);
            None
        } else {
            let watch = guard
                .entry(session.to_string())
                .and_modify(|w| {
                    w.mode = mode;
                    w.method = method.to_string();
                    w.state = Phase::Watching;
                    w.blocked_reason = None;
                    w.fix_turns = 0;
                    w.last_key = None;
                    w.auto_armed = false;
                    w.pushed = None;
                    w.queue = None;
                    w.hold = None;
                })
                .or_insert_with(|| Watch::new(session, mode, method, now));
            Some(watch.clone())
        };
        self.persist(&guard);
        result
    }
}

static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();

pub(crate) fn registry() -> &'static Arc<Registry> {
    REGISTRY.get_or_init(|| Arc::new(Registry::new(None)))
}

/// `fix` / `merge` while the session has an armed, unfinished watch.
pub(crate) fn mode_of(session: &str) -> Option<&'static str> {
    registry().get(session).filter(Watch::active).map(|w| {
        if w.mode == Mode::Merge {
            "merge"
        } else {
            "fix"
        }
    })
}

pub(crate) fn is_watching(session: &str) -> bool {
    mode_of(session).is_some()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/pr/watch"),
            get(watch_get).post(watch_set),
        )
        .route(&format!("{base}/api/pr/watches"), get(watches_list))
        .route(&format!("{base}/api/pr/queue"), get(queue_list))
}

#[derive(Deserialize)]
struct WatchQuery {
    session: String,
}

#[derive(Deserialize)]
struct WatchRequest {
    session: String,
    mode: Mode,
    #[serde(default)]
    method: Option<String>,
}

async fn watch_get(Query(query): Query<WatchQuery>) -> Response {
    json_response(&serde_json::json!({ "watch": registry().get(&query.session) }))
}

async fn queue_list() -> Response {
    json_response(&serde_json::json!({
        "queues": crate::serve_merge_queue::snapshot(&registry().all())
    }))
}

async fn watches_list() -> Response {
    json_response(&serde_json::json!({ "watches": registry().all() }))
}

async fn watch_set(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<WatchRequest>,
) -> Response {
    let method = request.method.as_deref().unwrap_or("squash");
    if !matches!(method, "squash" | "merge" | "rebase" | "auto") {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "method must be squash, merge, rebase or auto",
        );
    }
    if let Err((status, message)) = session_repo_root(&state, &request.session).await {
        return err_response(status, &message);
    }
    let watch = arm_watch(&state, &request.session, request.mode, method).await;
    json_response(&serde_json::json!({ "watch": watch }))
}

/// Arm (or, for `off`, clear) a session's watch; a watched session runs in the Auto profile.
pub(crate) async fn arm_watch(
    state: &DaemonState,
    session: &str,
    mode: Mode,
    method: &str,
) -> Option<Watch> {
    if mode != Mode::Off {
        if let Some(handle) = state.registry.get(session).await {
            let _ = handle
                .set_permission_mode(forge_types::PermissionMode::Auto)
                .await;
        }
    }
    let watch = registry().arm(session, mode, method, now_secs());
    state.registry.notify_fleet();
    watch
}

/// Arm (or clear) a session's watch without touching its permission mode, for sessions whose
/// work was seeded from untrusted text (a GitHub issue).
pub(crate) fn arm_watch_keeping_mode(
    state: &DaemonState,
    session: &str,
    mode: Mode,
    method: &str,
) -> Option<Watch> {
    let watch = registry().arm(session, mode, method, now_secs());
    state.registry.notify_fleet();
    watch
}

pub(crate) fn start(state: &Arc<DaemonState>) {
    if let Some(path) = forge_config::data_dir().map(|d| d.join("pr-watch.json")) {
        let _ = REGISTRY.set(Arc::new(Registry::new(Some(path))));
    }
    let state = state.clone();
    tokio::spawn(async move {
        let gh = Gh::system();
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            sweep(&state, &gh, now_secs()).await;
        }
    });
}

fn is_force_push(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    lower.contains("push")
        && (lower.contains("--force")
            || lower.contains("force-with-lease")
            || lower.contains("force-push")
            || lower.contains("force push")
            || lower
                .split_whitespace()
                .any(|w| w == "-f" || w.starts_with('+')))
}

async fn sweep(state: &Arc<DaemonState>, gh: &Gh, now: i64) {
    for watch in registry().all().into_iter().filter(Watch::active) {
        let Some(handle) = state.registry.get(&watch.session_id).await else {
            continue;
        };
        let snapshot = handle.snapshot_rx.borrow().snapshot.clone();
        if let Some(prompt) = snapshot.permission_prompt.as_deref() {
            if is_force_push(prompt) {
                let _ = handle
                    .input_tx
                    .send(crate::remote::RemoteInput::Allow {
                        yes: false,
                        seq: snapshot.prompt_seq,
                        note: None,
                        scope: None,
                    })
                    .await;
                let mut blocked = watch.clone();
                blocked.state = Phase::Blocked;
                blocked.blocked_reason =
                    Some("refused a force push; PR Watch only pushes normally".into());
                let announcement = settle_push(&mut blocked, false);
                registry().update(&watch.session_id, |w| *w = blocked.clone());
                state.registry.notify_fleet();
                announce(state, &watch.session_id, announcement).await;
            }
            continue;
        }
        if watch.last_checked.is_some_and(|t| now - t < POLL_SECS) {
            continue;
        }
        let Ok(root) = session_repo_root(state, &watch.session_id).await else {
            continue;
        };
        let free = !snapshot.busy && snapshot.question.is_none();
        let mut next = watch.clone();
        let all = registry().all();
        let outcome = advance(gh, &root, &mut next, all, free, now, None).await;
        let announcement = settle_push(&mut next, outcome.green);
        registry().update(&watch.session_id, |w| *w = next.clone());
        announce(state, &watch.session_id, announcement).await;
        if let Some(text) = outcome.prompt {
            let _ = handle
                .input_tx
                .send(crate::remote::RemoteInput::Prompt {
                    text,
                    attachments: None,
                })
                .await;
        }
        if outcome.merged {
            let _ = state
                .store
                .set_session_done_at(&watch.session_id, Some(now));
        }
        if outcome.changed {
            state.registry.notify_fleet();
        }
    }
}

/// One watch's turn: the merge queue's update step for a merge-mode watch, then PR Watch's poll.
/// A hand-off or a push settles the tick on its own; the poll waits for the next one.
pub(crate) async fn advance(
    gh: &Gh,
    root: &Path,
    watch: &mut Watch,
    all: Vec<Watch>,
    free: bool,
    now: i64,
    verify: Option<Vec<crate::serve_merge_queue::Verify>>,
) -> Outcome {
    if watch.mode != Mode::Merge {
        watch.hold = None;
        watch.queue = None;
        return poll_once(gh, root, watch, free, now).await;
    }
    let before = watch.clone();
    let pre = crate::serve_merge_queue::before_poll(root, watch, all, free, now, verify).await;
    if pre.acted {
        watch.last_checked = Some(now);
        return Outcome {
            prompt: pre.prompt,
            changed: true,
            ..Outcome::default()
        };
    }
    let mut out = poll_once(gh, root, watch, free, now).await;
    out.changed |= {
        let mut left = before;
        left.last_checked = watch.last_checked;
        left != *watch
    };
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) prompt: Option<String>,
    pub(crate) merged: bool,
    pub(crate) changed: bool,
    /// An open PR with every check passing and nothing left for the agent to fix.
    pub(crate) green: bool,
}

/// One push per state transition: what to say, and the key that remembers it was said.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Announcement {
    key: &'static str,
    pub(crate) message: String,
    pub(crate) high: bool,
}

/// The push owed for `watch`'s current state, or `None` when it was already sent (or nothing
/// push-worthy happened). Records the key in `watch.pushed` so the caller persists it with the
/// rest of the update: the push is at-most-once across a restart rather than at-least-once.
pub(crate) fn settle_push(watch: &mut Watch, green: bool) -> Option<Announcement> {
    let pr = watch
        .pr_number
        .map_or_else(|| "PR".to_string(), |n| format!("PR #{n}"));
    let (key, message, high) = match watch.state {
        Phase::Merged => ("merged", format!("{pr} merged"), false),
        Phase::Blocked => (
            "blocked",
            match watch.blocked_reason.as_deref() {
                Some(reason) => format!("{pr} blocked: {reason}"),
                None => format!("{pr} blocked"),
            },
            true,
        ),
        Phase::Watching if green && watch.mode == Mode::Fix => (
            "green",
            format!("{pr} is green \u{2014} waiting for your merge"),
            false,
        ),
        Phase::Watching => return None,
    };
    if watch.pushed.as_deref() == Some(key) {
        return None;
    }
    watch.pushed = Some(key.to_string());
    Some(Announcement { key, message, high })
}

/// Delivers through the same channels as the `push_notification` tool (Web Push, APNs, the
/// Anywhere hint, desktop); the session id is the deep link the notification opens.
async fn announce(state: &Arc<DaemonState>, session: &str, announcement: Option<Announcement>) {
    let Some(a) = announcement else { return };
    tracing::info!(session, transition = a.key, "PR Watch push");
    let request = forge_core::push_notify::PushRequest {
        message: a.message,
        title: Some("PR Watch".into()),
        priority: if a.high {
            forge_core::push_notify::Priority::High
        } else {
            forge_core::push_notify::Priority::Normal
        },
        session_id: session.to_string(),
    };
    crate::push_tool::channels_for(state, !cfg!(test))
        .deliver(&request)
        .await;
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Wait(Option<String>),
    Block(String),
    Fix { key: String, prompt: String },
    Merge,
    Merged,
    Green,
}

fn failing_lines(pr: &PrSummary) -> Vec<String> {
    let mut lines: Vec<String> = pr
        .checks
        .items
        .iter()
        .filter(|c| c.bucket == "failing")
        .map(|c| match &c.url {
            Some(url) => format!("- {} ({}): {url}", c.name, c.detail),
            None => format!("- {} ({})", c.name, c.detail),
        })
        .collect();
    lines.sort();
    lines
}

fn fix_prompt(pr: &PrSummary, failing: &[String], changes: bool, new_comments: usize) -> String {
    let mut text = format!(
        "PR Watch: pull request #{} ({}) needs attention.\n",
        pr.number, pr.url
    );
    if !failing.is_empty() {
        text.push_str("\nFailing checks:\n");
        text.push_str(&failing.join("\n"));
        text.push('\n');
    }
    if changes {
        text.push_str("\nA reviewer requested changes.\n");
    }
    if new_comments > 0 {
        text.push_str(&format!(
            "\n{new_comments} new comment(s) arrived; read them with `gh pr view {} --comments`.\n",
            pr.number
        ));
    }
    text.push_str(
        "\nFix the cause in this worktree, run the relevant checks locally, commit, and push \
         with a normal `git push`. Never force-push, never rewrite history, never delete the \
         branch. If you cannot fix it, say why and stop.",
    );
    text
}

fn decide(watch: &Watch, pr: &PrSummary, comments: &[String], free: bool) -> Decision {
    match pr.state.as_str() {
        "merged" => return Decision::Merged,
        "closed" => return Decision::Block("the PR was closed without merging".into()),
        _ => {}
    }
    if pr.draft {
        return Decision::Wait(Some("the PR is a draft".into()));
    }
    let conflicting = pr.mergeable == "conflicting" || pr.merge_state == "dirty";
    if watch.mode == Mode::Merge && watch.method == "auto" && !watch.auto_armed && !conflicting {
        if let Some(reason) = &watch.hold {
            return Decision::Wait(Some(reason.clone()));
        }
        return Decision::Merge;
    }
    if pr.checks.pending > 0 {
        return Decision::Wait(None);
    }
    let failing = pr.checks.failing > 0;
    let changes = pr.review_decision == "changes_requested";
    let fresh: Vec<&String> = comments
        .iter()
        .filter(|id| !watch.seen_comments.contains(id))
        .collect();
    if failing || changes || !fresh.is_empty() {
        let lines = failing_lines(pr);
        let mut ids: Vec<&str> = fresh.iter().map(|s| s.as_str()).collect();
        ids.sort_unstable();
        let key = format!(
            "{}|{}|{changes}|{}",
            pr.head_sha,
            lines.join(";"),
            ids.join(",")
        );
        if watch.last_key.as_deref() == Some(key.as_str()) {
            return Decision::Wait(Some("fix turn sent; waiting for a push".into()));
        }
        if !free {
            return Decision::Wait(None);
        }
        if watch.fix_turns >= MAX_FIX_TURNS {
            return Decision::Block(format!(
                "gave up after {MAX_FIX_TURNS} fix turns without a green PR"
            ));
        }
        return Decision::Fix {
            prompt: fix_prompt(pr, &lines, changes, fresh.len()),
            key,
        };
    }
    if watch.mode != Mode::Merge {
        return Decision::Green;
    }
    if let Some(reason) = &watch.hold {
        return Decision::Wait(Some(reason.clone()));
    }
    if conflicting {
        return Decision::Wait(Some("merge conflicts with the base branch".into()));
    }
    match pr.merge_state.as_str() {
        "clean" | "has_hooks" | "unstable" => Decision::Merge,
        "behind" => Decision::Wait(Some("the branch is behind its base".into())),
        "blocked" => Decision::Wait(Some("waiting for a required review or check".into())),
        other => Decision::Wait(Some(format!("merge state is {other}"))),
    }
}

fn comment_ids(value: &Value) -> Vec<String> {
    ["comments", "reviews"]
        .iter()
        .flat_map(|key| {
            value
                .get(*key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

pub(crate) async fn poll_once(
    gh: &Gh,
    root: &Path,
    watch: &mut Watch,
    free: bool,
    now: i64,
) -> Outcome {
    let before = watch.clone();
    watch.last_checked = Some(now);
    let fields = format!("{SUMMARY_FIELDS},comments,reviews");
    let raw = match gh.run(root, &["pr", "view", "--json", &fields], None).await {
        Ok(raw) => raw,
        Err(GhError::NoPr) => {
            watch.blocked_reason = Some("no pull request for this branch yet".into());
            return finish(&before, watch, Outcome::default());
        }
        Err(e) => {
            watch.blocked_reason = Some(e.message());
            return finish(&before, watch, Outcome::default());
        }
    };
    let Some(value) = serde_json::from_str::<Value>(&raw).ok() else {
        watch.blocked_reason = Some("unreadable gh output".into());
        return finish(&before, watch, Outcome::default());
    };
    let Some(pr) = parse_summary(&value) else {
        watch.blocked_reason = Some("unreadable gh output".into());
        return finish(&before, watch, Outcome::default());
    };
    let comments = comment_ids(&value);
    watch.pr_number = Some(pr.number);
    watch.pr_url = Some(pr.url.clone());
    watch.head_sha = Some(pr.head_sha.clone());
    watch.base_branch = Some(pr.base_ref.clone()).filter(|b| !b.is_empty());
    if before.last_checked.is_none() {
        watch.seen_comments = comments.clone();
    }
    if pr.checks.failing == 0 && pr.checks.pending == 0 {
        watch.fix_turns = 0;
    }
    let mut outcome = Outcome::default();
    match decide(watch, &pr, &comments, free) {
        Decision::Wait(reason) => watch.blocked_reason = reason,
        Decision::Green => {
            watch.blocked_reason = None;
            outcome.green = true;
        }
        Decision::Block(reason) => {
            watch.state = Phase::Blocked;
            watch.blocked_reason = Some(reason);
        }
        Decision::Fix { key, prompt } => {
            watch.last_key = Some(key);
            watch.seen_comments = comments;
            watch.fix_turns += 1;
            watch.blocked_reason = None;
            watch.last_action = Some(format!("fix turn {} sent", watch.fix_turns));
            outcome.prompt = Some(prompt);
        }
        Decision::Merged => {
            watch.state = Phase::Merged;
            watch.blocked_reason = None;
            watch.last_action = Some("merged".into());
            outcome.merged = true;
        }
        Decision::Merge => match merge(gh, root, watch, &pr).await {
            Ok(true) => {
                watch.state = Phase::Merged;
                watch.blocked_reason = None;
                watch.last_action = Some("merged".into());
                outcome.merged = true;
            }
            Ok(false) => {
                watch.auto_armed = true;
                watch.blocked_reason = None;
                watch.last_action = Some("auto-merge enabled".into());
            }
            Err(e) => {
                watch.state = Phase::Blocked;
                watch.blocked_reason = Some(format!("merge failed: {}", e.message()));
            }
        },
    }
    // A decision was reached and it is not green: the next green is a new transition. (A failed
    // poll returns earlier and leaves this alone, so a flaky `gh` cannot repeat the push.)
    if watch.state == Phase::Watching && !outcome.green {
        watch.pushed = None;
    }
    finish(&before, watch, outcome)
}

fn finish(before: &Watch, after: &Watch, mut outcome: Outcome) -> Outcome {
    let mut left = before.clone();
    left.last_checked = after.last_checked;
    outcome.changed = left != *after;
    outcome
}

async fn merge(gh: &Gh, root: &Path, watch: &Watch, pr: &PrSummary) -> Result<bool, GhError> {
    if watch.method == "auto" {
        let number = pr.number.to_string();
        gh.run(
            root,
            &[
                "pr",
                "merge",
                &number,
                "--auto",
                "--squash",
                "--match-head-commit",
                &pr.head_sha,
            ],
            None,
        )
        .await?;
        return Ok(false);
    }
    merge_in(
        gh,
        root,
        &MergeRequest {
            session: watch.session_id.clone(),
            number: pr.number,
            head_sha: pr.head_sha.clone(),
            method: Some(watch.method.clone()),
            delete_branch: false,
        },
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests;
