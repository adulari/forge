//! `forge dispatch` — plan and dispatch from a terminal, as a thin client of the daemon's dispatch
//! routes (`POST /api/dispatch`, `/api/dispatches[/{id}[/proposal|approve|revise|cancel]]`).
//!
//! It shares `forge attach`'s discovery and auth (loopback + the persisted daemon token) and never
//! touches the store: the daemon owns every dispatch, so the board, this command, `/dispatch` in
//! chat and a CLI-bridge coordinator (`mcp_serve::dispatch`) all see the same state. The HTTP half
//! here ([`Daemon`], [`DispatchView`]) is the one client those last two reuse.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use forge_core::dispatch::{dispatch_status, item_status, status_words};
use serde_json::{json, Value};

use crate::attach::{resolve_base_url, resolve_token};
use crate::cli::args::{DispatchCmd, DispatchModeArg};

/// How many dispatches an id prefix is resolved against — the route's own ceiling.
const LIST_LIMIT: usize = 100;
/// `/dispatch` runs on the chat render loop, so the whole request is bounded well below the point
/// where a frozen screen reads as a hang.
const CHAT_TIMEOUT: Duration = Duration::from_secs(4);
const CHAT_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// The fields of the daemon's `DispatchJson` a client renders. Deserialize-only and tolerant of
/// additions, like `attach::SessionInfo`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct DispatchView {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) coordinator_session_id: String,
    #[serde(default)]
    pub(crate) coordinator_title: String,
    #[serde(default)]
    pub(crate) cwd: String,
    #[serde(default)]
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) worktree: bool,
    #[serde(default)]
    pub(crate) permission_mode: Option<String>,
    #[serde(default)]
    pub(crate) max_running: u64,
    #[serde(default)]
    pub(crate) max_items: u64,
    #[serde(default)]
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) items: Vec<ItemView>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct ItemView {
    pub(crate) index: usize,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) depends_on: Vec<usize>,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
}

/// `POST /api/dispatch`'s reply.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct StartReply {
    pub(crate) dispatch_id: String,
    #[serde(default)]
    pub(crate) coordinator_session_id: String,
}

/// Why a daemon call failed, kept apart so each surface can say what to do about it: an
/// unreachable daemon needs starting, a refusal carries the daemon's own reason.
#[derive(Debug)]
pub(crate) enum DaemonError {
    Unreachable { base: String, reason: String },
    TokenRejected,
    Refused { status: u16, message: String },
    BadResponse(String),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::Unreachable { base, reason } => write!(
                f,
                "could not reach the forge serve daemon at {base} — is it running? \
                 (start it with `forge serve --local`)  [{reason}]"
            ),
            // Every route sits under `/<token>/`, so a bare 404 is a bad token or a daemon that
            // predates these routes; a known route's own 404 carries an `error` body instead.
            DaemonError::TokenRejected => write!(
                f,
                "daemon answered 404 — wrong --token, the daemon rotated it, or the daemon is \
                 older than `forge dispatch` (restart it)"
            ),
            DaemonError::Refused { message, .. } => f.write_str(message),
            DaemonError::BadResponse(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DaemonError {}

/// One daemon endpoint plus its token.
pub(crate) struct Daemon {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Daemon {
    pub(crate) fn new(http: reqwest::Client, base: String, token: String) -> Self {
        Daemon {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}/api/{path}", self.base, self.token)
    }

    async fn call(&self, request: reqwest::RequestBuilder) -> Result<Value, DaemonError> {
        let resp = request.send().await.map_err(|e| DaemonError::Unreachable {
            base: self.base.clone(),
            reason: e.to_string(),
        })?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| {
            DaemonError::BadResponse(format!("could not read the daemon's reply: {e}"))
        })?;
        let parsed = serde_json::from_str::<Value>(&body).ok();
        if status.is_success() {
            return parsed.ok_or_else(|| {
                DaemonError::BadResponse(format!(
                    "the daemon's reply was not JSON: {}",
                    forge_types::truncate_ellipsis(&body, 200)
                ))
            });
        }
        let reason = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string);
        match reason {
            Some(message) => Err(DaemonError::Refused {
                status: status.as_u16(),
                message,
            }),
            None if status == reqwest::StatusCode::NOT_FOUND => Err(DaemonError::TokenRejected),
            None => Err(DaemonError::Refused {
                status: status.as_u16(),
                message: if body.trim().is_empty() {
                    format!("daemon returned {status}")
                } else {
                    format!("daemon returned {status}: {}", body.trim())
                },
            }),
        }
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Value, DaemonError> {
        self.call(self.http.get(self.url(path))).await
    }

    pub(crate) async fn post(&self, path: &str, body: &Value) -> Result<Value, DaemonError> {
        self.call(self.http.post(self.url(path)).json(body)).await
    }

    pub(crate) async fn list(&self, limit: usize) -> Result<Vec<DispatchView>, DaemonError> {
        decode(self.get(&format!("dispatches?limit={limit}")).await?)
    }

    pub(crate) async fn resolve_id(&self, needle: &str) -> Result<String> {
        let list = self.list(LIST_LIMIT).await?;
        resolve_dispatch_id(&list, needle)
    }
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, DaemonError> {
    serde_json::from_value(value).map_err(|e| {
        DaemonError::BadResponse(format!("the daemon's reply was not the expected JSON: {e}"))
    })
}

pub(crate) async fn dispatch_cmd(
    cmd: DispatchCmd,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let daemon = Daemon::new(
        reqwest::Client::new(),
        resolve_base_url(url),
        resolve_token(token)?,
    );
    match cmd {
        DispatchCmd::Start {
            prompt,
            cwd,
            no_worktree,
            mode,
            parallel,
            max_items,
            model,
        } => {
            let cwd = resolve_cwd(cwd)?;
            let body = start_body(
                &prompt,
                &cwd,
                no_worktree,
                mode,
                parallel,
                max_items,
                model.as_deref(),
            )?;
            let reply: StartReply = decode(daemon.post("dispatch", &body).await?)?;
            println!("{}", started_text(&reply));
        }
        DispatchCmd::List => {
            let list = daemon.list(LIST_LIMIT).await?;
            println!("{}", list_text(&list, unix_now()));
        }
        DispatchCmd::Show { id } => {
            let id = daemon.resolve_id(&id).await?;
            let dispatch: DispatchView = decode(daemon.get(&format!("dispatches/{id}")).await?)?;
            println!("{}", show_text(&dispatch));
        }
        DispatchCmd::Approve { id, only } => {
            let selected = only.as_deref().map(parse_only).transpose()?;
            let id = daemon.resolve_id(&id).await?;
            let body = match selected {
                Some(selected) => json!({ "selected": selected }),
                None => json!({}),
            };
            let dispatch: DispatchView = decode(
                daemon
                    .post(&format!("dispatches/{id}/approve"), &body)
                    .await?,
            )?;
            println!("{}", approved_text(&dispatch));
        }
        DispatchCmd::Revise { id, feedback } => {
            let feedback = feedback.trim();
            if feedback.is_empty() {
                bail!("feedback must not be empty: say what should change in the split");
            }
            let id = daemon.resolve_id(&id).await?;
            let body = json!({ "feedback": feedback });
            let dispatch: DispatchView = decode(
                daemon
                    .post(&format!("dispatches/{id}/revise"), &body)
                    .await?,
            )?;
            println!("{}", revised_text(&dispatch));
        }
        DispatchCmd::Cancel { id } => {
            let id = daemon.resolve_id(&id).await?;
            let dispatch: DispatchView = decode(
                daemon
                    .post(&format!("dispatches/{id}/cancel"), &json!({}))
                    .await?,
            )?;
            println!("{}", cancelled_text(&dispatch));
        }
        DispatchCmd::Merge { id } => {
            let id = daemon.resolve_id(&id).await?;
            let report: MergeReport = decode(
                daemon
                    .post(&format!("dispatches/{id}/merge"), &json!({}))
                    .await?,
            )?;
            println!("{}", merge_text(&report));
        }
    }
    Ok(())
}

/// `POST /api/dispatches/{id}/merge`'s report.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct MergeReport {
    #[serde(default)]
    merged: Vec<MergedView>,
    #[serde(default)]
    stopped_at: Option<StoppedView>,
    #[serde(default)]
    remaining: Vec<usize>,
    #[serde(default)]
    base_branch: Option<String>,
    #[serde(default)]
    dispatch: DispatchView,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct MergedView {
    index: usize,
    #[serde(default)]
    title: String,
    #[serde(default)]
    commit: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct StoppedView {
    index: usize,
    #[serde(default)]
    title: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    conflicts: Vec<String>,
}

fn merge_text(r: &MergeReport) -> String {
    let merged = r.merged.len();
    let total = merged + usize::from(r.stopped_at.is_some()) + r.remaining.len();
    let into = r
        .base_branch
        .as_deref()
        .map(|b| format!(" into {b}"))
        .unwrap_or_default();
    let count = if r.stopped_at.is_some() {
        format!("{merged} of {total} items")
    } else {
        format!("{merged} item{}", if merged == 1 { "" } else { "s" })
    };
    let mut out = format!(
        "⚒ merged {count} of dispatch {}{into}",
        short(&r.dispatch.id)
    );
    let width = r.merged.iter().map(|m| m.title.chars().count()).max();
    for m in &r.merged {
        let commit = m
            .commit
            .as_deref()
            .map_or("nothing to commit", |sha| short(sha));
        out.push_str(&format!(
            "\n  {:>2}. {:<w$}  {commit}",
            m.index,
            m.title,
            w = width.unwrap_or(0)
        ));
    }
    if let Some(s) = &r.stopped_at {
        out.push_str(&format!(
            "\nstopped at {}. {}: {}",
            s.index, s.title, s.reason
        ));
        if !s.conflicts.is_empty() {
            out.push_str(&format!("\n  conflicts: {}", s.conflicts.join(", ")));
        }
        if !r.remaining.is_empty() {
            let rest: Vec<String> = r.remaining.iter().map(usize::to_string).collect();
            out.push_str(&format!("\n  not merged yet: {}", rest.join(", ")));
        }
    }
    out
}

/// `/dispatch <request>` from chat: start a dispatch for this session's workspace and return the
/// one note the transcript shows. Never an `Err` — every outcome is something to tell the user.
pub(crate) async fn chat_dispatch_note(prompt: &str, cwd: &Path) -> String {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return "usage: /dispatch <request>".to_string();
    }
    let token = match resolve_token(None) {
        Ok(token) => token,
        Err(e) => {
            return format!("⚠ dispatch not started: {e} — start it with: forge serve --local")
        }
    };
    let http = match reqwest::Client::builder()
        .connect_timeout(CHAT_CONNECT_TIMEOUT)
        .timeout(CHAT_TIMEOUT)
        .build()
    {
        Ok(http) => http,
        Err(e) => return format!("⚠ dispatch not started: {e}"),
    };
    let daemon = Daemon::new(http, resolve_base_url(None), token);
    let body = json!({ "prompt": prompt, "cwd": cwd.display().to_string() });
    let result = match daemon.post("dispatch", &body).await {
        Ok(value) => decode::<StartReply>(value),
        Err(e) => Err(e),
    };
    chat_note(result)
}

fn chat_note(result: Result<StartReply, DaemonError>) -> String {
    match result {
        Ok(reply) => format!(
            "◆ dispatch {} started — watch and approve it in forge board",
            short(&reply.dispatch_id)
        ),
        Err(DaemonError::Unreachable { base, reason }) => format!(
            "⚠ could not reach the forge serve daemon at {base} ({reason}) — start it with: forge serve --local"
        ),
        Err(e) => format!("⚠ dispatch not started: {e}"),
    }
}

fn resolve_cwd(cwd: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match cwd {
        Some(dir) => dir,
        None => std::env::current_dir().context("reading the current directory")?,
    };
    std::fs::canonicalize(&dir).with_context(|| format!("--cwd {}", dir.display()))
}

/// The `POST /api/dispatch` body. Unset options are left out so the daemon's defaults apply.
fn start_body(
    prompt: &str,
    cwd: &Path,
    no_worktree: bool,
    mode: Option<DispatchModeArg>,
    parallel: Option<u32>,
    max_items: Option<u32>,
    model: Option<&str>,
) -> Result<Value> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        bail!("the request must not be empty: forge dispatch start \"<what to do>\"");
    }
    let mut body = json!({
        "prompt": prompt,
        "cwd": cwd.display().to_string(),
        "worktree": !no_worktree,
    });
    if let Some(mode) = mode {
        body["mode"] = json!(mode.wire());
    }
    if let Some(parallel) = parallel {
        body["max_running"] = json!(parallel);
    }
    if let Some(max_items) = max_items {
        body["max_items"] = json!(max_items);
    }
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        body["model"] = json!(model);
    }
    Ok(body)
}

/// `--only 1,3` → `[1, 3]` (sorted, deduplicated).
fn parse_only(raw: &str) -> Result<Vec<usize>> {
    let mut numbers = Vec::new();
    for part in raw.split(',').map(str::trim) {
        match part.parse::<usize>() {
            Ok(n) if n > 0 => numbers.push(n),
            _ => bail!("--only takes item numbers separated by commas, like 1,3 (got {part:?})"),
        }
    }
    numbers.sort_unstable();
    numbers.dedup();
    Ok(numbers)
}

/// Resolve a full dispatch id from an exact id or a unique prefix, like `forge attach`.
fn resolve_dispatch_id(dispatches: &[DispatchView], needle: &str) -> Result<String> {
    let needle = needle.trim();
    if let Some(exact) = dispatches.iter().find(|d| d.id == needle) {
        return Ok(exact.id.clone());
    }
    let matches: Vec<&DispatchView> = dispatches
        .iter()
        .filter(|d| !needle.is_empty() && d.id.starts_with(needle))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id.clone()),
        [] if dispatches.is_empty() => bail!("there are no dispatches on this daemon"),
        [] => {
            let ids: Vec<&str> = dispatches.iter().map(|d| short(&d.id)).collect();
            bail!("no dispatch matches {needle:?}. known: {}", ids.join(", "))
        }
        many => {
            let ids: Vec<&str> = many.iter().map(|d| d.id.as_str()).collect();
            bail!("{needle:?} is ambiguous — matches: {}", ids.join(", "))
        }
    }
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn format_age(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3_600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3_600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// `(finished, total)` items.
fn progress(dispatch: &DispatchView) -> (usize, usize) {
    let done = dispatch
        .items
        .iter()
        .filter(|i| item_status::is_terminal(&i.status))
        .count();
    (done, dispatch.items.len())
}

fn count(dispatch: &DispatchView, status: &str) -> usize {
    dispatch.items.iter().filter(|i| i.status == status).count()
}

fn started_text(reply: &StartReply) -> String {
    let id8 = short(&reply.dispatch_id);
    format!(
        "⚒ dispatch {id8} started — the coordinator ({}) is reading the project and splitting the work.\n  \
         review and approve it in `forge board`, or: forge dispatch show {id8}",
        short(&reply.coordinator_session_id)
    )
}

fn list_text(dispatches: &[DispatchView], now: i64) -> String {
    if dispatches.is_empty() {
        return "no dispatches on this daemon. start one with: forge dispatch start \"<request>\""
            .to_string();
    }
    let mut sorted: Vec<&DispatchView> = dispatches.iter().collect();
    sorted.sort_by_key(|d| std::cmp::Reverse(d.created_at));
    sorted
        .iter()
        .map(|d| {
            let (done, total) = progress(d);
            let first_line = d.prompt.lines().next().unwrap_or("").trim();
            format!(
                "{}  {:<9}  {:>5}  {:>8}  {}",
                short(&d.id),
                d.status,
                format!("{done}/{total}"),
                format_age(now - d.created_at),
                forge_types::truncate_ellipsis(first_line, 72)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn show_text(d: &DispatchView) -> String {
    let id8 = short(&d.id);
    let (done, total) = progress(d);
    let mut out = format!(
        "⚒ dispatch {id8}  {}  ({done}/{total} finished)\n",
        d.status
    );
    let title = if d.coordinator_title.is_empty() {
        String::new()
    } else {
        format!("  {}", d.coordinator_title)
    };
    out.push_str(&format!(
        "coordinator  {}{title}\n",
        short(&d.coordinator_session_id)
    ));
    let isolation = if d.worktree {
        "a worktree per item"
    } else {
        "one shared working directory"
    };
    out.push_str(&format!(
        "project      {}  ({isolation}, {} mode, up to {} at a time)\n",
        d.cwd,
        d.permission_mode.as_deref().unwrap_or("accept-edits"),
        d.max_running
    ));
    out.push_str("\nrequest\n");
    out.push_str(&indent(&d.prompt));
    if !d.summary.trim().is_empty() {
        out.push_str("\nsummary\n");
        out.push_str(&indent(&d.summary));
    }
    if d.items.is_empty() {
        if d.status == dispatch_status::PLANNING {
            out.push_str("\nno split proposed yet — the coordinator is still working on it.\n");
        }
    } else {
        out.push_str("\nitems\n");
        for item in &d.items {
            out.push_str(&item_row(item));
            out.push('\n');
        }
    }
    if d.status == dispatch_status::PROPOSED {
        out.push_str(&format!("\napprove all   forge dispatch approve {id8}\n"));
        if d.items.len() > 1 {
            let some: Vec<String> = d
                .items
                .iter()
                .take(d.items.len() - 1)
                .map(|i| i.index.to_string())
                .collect();
            out.push_str(&format!(
                "approve some  forge dispatch approve {id8} --only {}\n",
                some.join(",")
            ));
        }
        out.push_str(&format!(
            "revise        forge dispatch revise {id8} \"<what to change>\"\n"
        ));
        out.push_str(&format!("cancel        forge dispatch cancel {id8}\n"));
    }
    out.trim_end().to_string()
}

fn item_row(item: &ItemView) -> String {
    let mut row = format!(
        "  {:>2}. {:<26}  {}",
        item.index,
        status_words(&item.status),
        item.title
    );
    if !item.depends_on.is_empty() {
        let deps: Vec<String> = item.depends_on.iter().map(usize::to_string).collect();
        row.push_str(&format!("  (after {})", deps.join(", ")));
    }
    if let Some(session) = item.session_id.as_deref().filter(|s| !s.is_empty()) {
        row.push_str(&format!("  session {}", short(session)));
    }
    row
}

fn indent(text: &str) -> String {
    text.trim()
        .lines()
        .map(|l| format!("  {l}\n"))
        .collect::<String>()
}

fn approved_text(d: &DispatchView) -> String {
    let mut parts = vec![
        format!("{} started", count(d, item_status::RUNNING)),
        format!("{} waiting to start", count(d, item_status::QUEUED)),
    ];
    let skipped = count(d, item_status::SKIPPED);
    if skipped > 0 {
        parts.push(format!("{skipped} not selected"));
    }
    let cancelled = count(d, item_status::CANCELLED);
    if cancelled > 0 {
        parts.push(format!(
            "{cancelled} cancelled (they depend on an item that was not selected)"
        ));
    }
    format!(
        "⚒ approved dispatch {} — {}. follow it in `forge board`",
        short(&d.id),
        parts.join(", ")
    )
}

fn revised_text(d: &DispatchView) -> String {
    let id8 = short(&d.id);
    format!(
        "⚒ sent your feedback to the coordinator of dispatch {id8} — it will propose a new split \
         (forge dispatch show {id8})"
    )
}

fn cancelled_text(d: &DispatchView) -> String {
    let running = match count(d, item_status::RUNNING) {
        0 => String::new(),
        1 => "; the 1 session already running keeps running".to_string(),
        n => format!("; the {n} sessions already running keep running"),
    };
    format!(
        "⚒ cancelled dispatch {} — nothing new starts{running}",
        short(&d.id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{Cli, Command};
    use clap::Parser;

    fn parse(args: &[&str]) -> (DispatchCmd, Option<String>, Option<String>) {
        let cli = Cli::try_parse_from(std::iter::once("forge").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?} should parse: {e}"));
        match cli.command {
            Some(Command::Dispatch { cmd, url, token }) => (cmd, url, token),
            _ => panic!("{args:?} is not `forge dispatch`"),
        }
    }

    fn rejects(args: &[&str]) -> bool {
        Cli::try_parse_from(std::iter::once("forge").chain(args.iter().copied())).is_err()
    }

    #[test]
    fn start_parses_every_flag() {
        let (cmd, url, token) = parse(&[
            "dispatch",
            "start",
            "split the parser work",
            "--cwd",
            "/repo",
            "--no-worktree",
            "--mode",
            "bypass",
            "--parallel",
            "3",
            "--max-items",
            "5",
            "--model",
            "anthropic::claude-opus-5",
            "--url",
            "http://127.0.0.1:9000",
            "--token",
            "tok",
        ]);
        let DispatchCmd::Start {
            prompt,
            cwd,
            no_worktree,
            mode,
            parallel,
            max_items,
            model,
        } = cmd
        else {
            panic!("not start");
        };
        assert_eq!(prompt, "split the parser work");
        assert_eq!(cwd, Some(PathBuf::from("/repo")));
        assert!(no_worktree);
        assert_eq!(mode, Some(DispatchModeArg::Bypass));
        assert_eq!(parallel, Some(3));
        assert_eq!(max_items, Some(5));
        assert_eq!(model.as_deref(), Some("anthropic::claude-opus-5"));
        assert_eq!(url.as_deref(), Some("http://127.0.0.1:9000"));
        assert_eq!(token.as_deref(), Some("tok"));
    }

    #[test]
    fn start_defaults_leave_everything_to_the_daemon() {
        let (cmd, url, token) = parse(&["dispatch", "start", "do it"]);
        let DispatchCmd::Start {
            cwd,
            no_worktree,
            mode,
            parallel,
            max_items,
            model,
            ..
        } = cmd
        else {
            panic!("not start");
        };
        assert!(cwd.is_none() && !no_worktree && mode.is_none());
        assert!(parallel.is_none() && max_items.is_none() && model.is_none());
        assert!(url.is_none() && token.is_none());
    }

    #[test]
    fn start_rejects_out_of_range_and_unknown_values() {
        assert!(rejects(&["dispatch", "start"]));
        assert!(rejects(&["dispatch", "start", "x", "--parallel", "0"]));
        assert!(rejects(&["dispatch", "start", "x", "--parallel", "9"]));
        assert!(rejects(&["dispatch", "start", "x", "--max-items", "13"]));
        assert!(rejects(&["dispatch", "start", "x", "--mode", "plan"]));
        assert!(rejects(&["dispatch"]));
    }

    #[test]
    fn accept_edits_mode_parses_and_maps_to_the_wire_name() {
        let (cmd, _, _) = parse(&["dispatch", "start", "x", "--mode", "accept-edits"]);
        let DispatchCmd::Start { mode, .. } = cmd else {
            panic!("not start");
        };
        assert_eq!(mode.map(DispatchModeArg::wire), Some("accept-edits"));
        assert_eq!(DispatchModeArg::Default.wire(), "default");
        assert_eq!(DispatchModeArg::Bypass.wire(), "bypass");
    }

    #[test]
    fn the_other_subcommands_parse() {
        assert!(matches!(parse(&["dispatch", "list"]).0, DispatchCmd::List));
        assert!(matches!(
            parse(&["dispatch", "show", "ab12"]).0,
            DispatchCmd::Show { id } if id == "ab12"
        ));
        assert!(matches!(
            parse(&["dispatch", "approve", "ab12"]).0,
            DispatchCmd::Approve { id, only: None } if id == "ab12"
        ));
        assert!(matches!(
            parse(&["dispatch", "approve", "ab12", "--only", "1,3"]).0,
            DispatchCmd::Approve { only: Some(o), .. } if o == "1,3"
        ));
        assert!(matches!(
            parse(&["dispatch", "revise", "ab12", "fewer items"]).0,
            DispatchCmd::Revise { id, feedback } if id == "ab12" && feedback == "fewer items"
        ));
        assert!(matches!(
            parse(&["dispatch", "merge", "ab12"]).0,
            DispatchCmd::Merge { id } if id == "ab12"
        ));
        assert!(rejects(&["dispatch", "merge"]));
        let (cmd, _, token) = parse(&["dispatch", "cancel", "ab12", "--token", "t"]);
        assert!(matches!(cmd, DispatchCmd::Cancel { id } if id == "ab12"));
        assert_eq!(token.as_deref(), Some("t"));
        assert!(rejects(&["dispatch", "revise", "ab12"]));
    }

    #[test]
    fn start_body_defaults() {
        let body = start_body(
            "  do it  ",
            Path::new("/repo"),
            false,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            body,
            json!({ "prompt": "do it", "cwd": "/repo", "worktree": true })
        );
    }

    #[test]
    fn start_body_with_every_option() {
        let body = start_body(
            "do it",
            Path::new("/repo"),
            true,
            Some(DispatchModeArg::Default),
            Some(2),
            Some(6),
            Some("mock"),
        )
        .unwrap();
        assert_eq!(
            body,
            json!({
                "prompt": "do it", "cwd": "/repo", "worktree": false, "mode": "default",
                "max_running": 2, "max_items": 6, "model": "mock"
            })
        );
        assert!(start_body("   ", Path::new("/repo"), false, None, None, None, None).is_err());
    }

    #[test]
    fn only_parses_numbers_and_rejects_the_rest() {
        assert_eq!(parse_only("1,3").unwrap(), vec![1, 3]);
        assert_eq!(parse_only(" 3, 1 ,3").unwrap(), vec![1, 3]);
        assert_eq!(parse_only("2").unwrap(), vec![2]);
        for bad in ["", "0", "a", "1,,2", "-1", "1;2"] {
            let err = parse_only(bad).unwrap_err().to_string();
            assert!(err.contains("--only"), "{bad}: {err}");
        }
    }

    fn view(id: &str, status: &str, created_at: i64) -> DispatchView {
        DispatchView {
            id: id.into(),
            status: status.into(),
            created_at,
            ..Default::default()
        }
    }

    #[test]
    fn dispatch_ids_resolve_exactly_or_by_unique_prefix() {
        let list = vec![
            view("aaa11111-x", "running", 1),
            view("aab22222-y", "proposed", 2),
        ];
        assert_eq!(
            resolve_dispatch_id(&list, "aaa11111-x").unwrap(),
            "aaa11111-x"
        );
        assert_eq!(resolve_dispatch_id(&list, "aab").unwrap(), "aab22222-y");
        let ambiguous = resolve_dispatch_id(&list, "aa").unwrap_err().to_string();
        assert!(ambiguous.contains("ambiguous"), "{ambiguous}");
        let absent = resolve_dispatch_id(&list, "zzz").unwrap_err().to_string();
        assert!(absent.contains("no dispatch matches"), "{absent}");
        assert!(resolve_dispatch_id(&list, "").is_err());
        let none = resolve_dispatch_id(&[], "aaa").unwrap_err().to_string();
        assert!(none.contains("no dispatches"), "{none}");
    }

    #[test]
    fn start_output_names_both_ids_and_the_next_step() {
        let reply = StartReply {
            dispatch_id: "d1234567-abcd".into(),
            coordinator_session_id: "c7654321-dcba".into(),
        };
        assert_eq!(
            started_text(&reply),
            "⚒ dispatch d1234567 started — the coordinator (c7654321) is reading the project and \
             splitting the work.\n  review and approve it in `forge board`, or: forge dispatch show d1234567"
        );
    }

    fn item(
        index: usize,
        title: &str,
        status: &str,
        deps: &[usize],
        session: Option<&str>,
    ) -> ItemView {
        ItemView {
            index,
            title: title.into(),
            depends_on: deps.to_vec(),
            status: status.into(),
            session_id: session.map(str::to_string),
        }
    }

    #[test]
    fn list_is_newest_first_with_progress_age_and_first_line() {
        let mut old = view("old00000-1", "done", 1_000);
        old.prompt = "first request\nmore detail".into();
        old.items = vec![item(1, "a", "succeeded", &[], None)];
        let mut new = view("new00000-2", "running", 9_940);
        new.prompt = "second request".into();
        new.items = vec![
            item(1, "a", "running", &[], None),
            item(2, "b", "failed", &[], None),
        ];
        let text = list_text(&[old, new], 10_000);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("new00000  running"), "{text}");
        assert!(
            lines[0].contains("1/2") && lines[0].contains("1m ago"),
            "{text}"
        );
        assert!(lines[0].ends_with("second request"), "{text}");
        assert!(
            lines[1].contains("1/1") && lines[1].contains("2h ago"),
            "{text}"
        );
        assert!(lines[1].ends_with("first request"), "{text}");
        assert!(list_text(&[], 0).contains("forge dispatch start"));
    }

    #[test]
    fn ages_read_in_the_largest_whole_unit() {
        assert_eq!(format_age(-5), "0s ago");
        assert_eq!(format_age(59), "59s ago");
        assert_eq!(format_age(3_599), "59m ago");
        assert_eq!(format_age(7_200), "2h ago");
        assert_eq!(format_age(200_000), "2d ago");
    }

    #[test]
    fn a_proposed_dispatch_shows_its_items_and_the_exact_commands() {
        let mut d = view("d1234567-abcd", "proposed", 0);
        d.coordinator_session_id = "c7654321-dcba".into();
        d.coordinator_title = "Dispatch: split it".into();
        d.cwd = "/repo".into();
        d.prompt = "split it".into();
        d.summary = "Two parts.".into();
        d.worktree = true;
        d.max_running = 4;
        d.items = vec![
            item(1, "Notes file", "proposed", &[], None),
            item(2, "Tasks", "proposed", &[], None),
            item(3, "Summary", "proposed", &[1, 2], None),
        ];
        let text = show_text(&d);
        assert!(
            text.starts_with("⚒ dispatch d1234567  proposed  (0/3 finished)"),
            "{text}"
        );
        assert!(
            text.contains("coordinator  c7654321  Dispatch: split it"),
            "{text}"
        );
        assert!(text.contains("request\n  split it"), "{text}");
        assert!(text.contains("summary\n  Two parts."), "{text}");
        assert!(
            text.contains("   3. proposed                    Summary  (after 1, 2)"),
            "{text}"
        );
        assert!(
            text.contains("approve all   forge dispatch approve d1234567"),
            "{text}"
        );
        assert!(
            text.contains("forge dispatch approve d1234567 --only 1,2"),
            "{text}"
        );
        assert!(
            text.contains("forge dispatch revise d1234567 \"<what to change>\""),
            "{text}"
        );
        assert!(
            text.ends_with("cancel        forge dispatch cancel d1234567"),
            "{text}"
        );
    }

    #[test]
    fn a_running_dispatch_shows_worker_sessions_and_no_approval_commands() {
        let mut d = view("d1234567-abcd", "running", 0);
        d.items = vec![
            item(1, "Notes file", "succeeded", &[], Some("w1111111-zz")),
            item(2, "Tasks", "queued", &[1], None),
        ];
        let text = show_text(&d);
        assert!(text.contains("(1/2 finished)"), "{text}");
        assert!(text.contains("succeeded"), "{text}");
        assert!(text.contains("Notes file  session w1111111"), "{text}");
        assert!(text.contains("waiting to start"), "{text}");
        assert!(!text.contains("forge dispatch approve"), "{text}");
    }

    #[test]
    fn action_results_summarize_the_returned_dispatch() {
        let mut d = view("d1234567-abcd", "running", 0);
        d.items = vec![
            item(1, "a", "running", &[], None),
            item(2, "b", "queued", &[1], None),
            item(3, "c", "skipped", &[], None),
            item(4, "d", "cancelled", &[3], None),
        ];
        assert_eq!(
            approved_text(&d),
            "⚒ approved dispatch d1234567 — 1 started, 1 waiting to start, 1 not selected, 1 \
             cancelled (they depend on an item that was not selected). follow it in `forge board`"
        );
        assert_eq!(
            cancelled_text(&d),
            "⚒ cancelled dispatch d1234567 — nothing new starts; the 1 session already running keeps running"
        );
        assert!(revised_text(&d).contains("forge dispatch show d1234567"));
    }

    #[test]
    fn chat_notes_cover_success_unreachable_and_refusal() {
        let ok = chat_note(Ok(StartReply {
            dispatch_id: "d1234567-abcd".into(),
            coordinator_session_id: String::new(),
        }));
        assert_eq!(
            ok,
            "◆ dispatch d1234567 started — watch and approve it in forge board"
        );
        let down = chat_note(Err(DaemonError::Unreachable {
            base: "http://127.0.0.1:7420".into(),
            reason: "connection refused".into(),
        }));
        assert!(down.contains("connection refused"), "{down}");
        assert!(
            down.ends_with("start it with: forge serve --local"),
            "{down}"
        );
        let refused = chat_note(Err(DaemonError::Refused {
            status: 400,
            message: "worktree: /tmp is not a git repository".into(),
        }));
        assert!(
            refused.contains("worktree: /tmp is not a git repository"),
            "{refused}"
        );
    }

    #[test]
    fn a_merge_report_lists_each_commit_then_the_stop_reason() {
        let full: MergeReport = serde_json::from_value(json!({
            "merged": [
                {"index": 1, "title": "Notes file", "commit": "a1b2c3d4e5f6"},
                {"index": 2, "title": "Tasks", "commit": null}
            ],
            "stopped_at": null, "remaining": [], "base_branch": "main",
            "dispatch": {"id": "d1234567-abcd"}
        }))
        .unwrap();
        assert_eq!(
            merge_text(&full),
            "⚒ merged 2 items of dispatch d1234567 into main\n   \
             1. Notes file  a1b2c3d4\n   2. Tasks       nothing to commit"
        );

        let partial: MergeReport = serde_json::from_value(json!({
            "merged": [{"index": 1, "title": "Notes file", "commit": "a1b2c3d4e5f6"}],
            "stopped_at": {"index": 2, "title": "Tasks", "reason": "merge conflicts", "conflicts": ["f.txt", "g.txt"]},
            "remaining": [3], "base_branch": null,
            "dispatch": {"id": "d1234567-abcd"}
        }))
        .unwrap();
        assert_eq!(
            merge_text(&partial),
            "⚒ merged 1 of 3 items of dispatch d1234567\n   1. Notes file  a1b2c3d4\n\
             stopped at 2. Tasks: merge conflicts\n  conflicts: f.txt, g.txt\n  not merged yet: 3"
        );
    }

    #[tokio::test]
    async fn an_empty_chat_request_prints_the_usage() {
        assert_eq!(
            chat_dispatch_note("   ", Path::new("/repo")).await,
            "usage: /dispatch <request>"
        );
    }
}
