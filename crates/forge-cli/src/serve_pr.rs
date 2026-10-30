//! `forge serve`'s pull-request surface, a thin layer over the `gh` CLI.
//!
//! Routes (token-scoped under `{base}`, session-addressed like the git dock; a client never names
//! a directory):
//! - `GET  /api/pr/status?session=`                      the checked-out branch's PR, checks rolled up
//! - `GET  /api/pr/list?session=[&state=open|closed|merged|all]`  the repository's PRs
//! - `GET  /api/pr/detail?session=&number=`              summary + body, comments, commits, files
//! - `GET  /api/pr/diff?session=&number=[&commit=][&ignore_whitespace=]`  the PR's patch as diff rows
//! - `POST /api/pr/create`  `{session, title, body?, base?, draft?}` push + open (the `open_pr` checks)
//! - `POST /api/pr/comment` `{session, number, body}`
//! - `POST /api/pr/merge`   `{session, number, head_sha, method?, delete_branch?}` bound to the SHA seen
//!
//! `gh` runs without a TTY, with prompts disabled and under a timeout. A missing or signed-out
//! `gh` is reported as data (`available: false`) on the read routes so a client can show a quiet
//! "connect GitHub" state instead of an error toast.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use axum::extract::{Json, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::serve::{err_response, git_stdout, json_response, DaemonState};
use crate::serve_git::{parse_patch, session_repo_root};

pub(crate) use gh::{fail, Gh, GhError};
#[cfg(test)]
use model::summarize_checks;
pub(crate) use model::{parse_summary, PrSummary, SUMMARY_FIELDS};

const MAX_COMMENT_BYTES: usize = 60_000;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/pr/status"), get(pr_status))
        .route(&format!("{base}/api/pr/list"), get(pr_list))
        .route(&format!("{base}/api/pr/detail"), get(pr_detail))
        .route(&format!("{base}/api/pr/diff"), get(pr_diff))
        .route(&format!("{base}/api/pr/create"), post(pr_create))
        .route(&format!("{base}/api/pr/comment"), post(pr_comment))
        .route(&format!("{base}/api/pr/merge"), post(pr_merge))
        .merge(crate::serve_pr_watch::routes(base))
}

fn http_fail((status, message): (axum::http::StatusCode, String)) -> Response {
    err_response(status, &message)
}

fn parse_json(raw: &str) -> Result<Value, GhError> {
    serde_json::from_str(raw).map_err(|e| GhError::Failed(format!("unreadable gh output: {e}")))
}

#[derive(Deserialize)]
pub(crate) struct SessionQuery {
    session: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    ignore_whitespace: Option<bool>,
}

#[derive(Serialize)]
struct StatusResponse {
    available: bool,
    error: Option<String>,
    branch: Option<String>,
    pr: Option<PrSummary>,
}

async fn pr_status(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SessionQuery>,
) -> Response {
    let root = match session_repo_root(&state, &query.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    json_response(&status_in(&Gh::system(), &root).await)
}

async fn status_in(gh: &Gh, root: &Path) -> StatusResponse {
    let branch = git_stdout(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .filter(|b| !b.is_empty());
    let result = gh
        .run(root, &["pr", "view", "--json", SUMMARY_FIELDS], None)
        .await;
    let (available, error, pr) = match result {
        Ok(raw) => match parse_json(&raw) {
            Ok(value) => (true, None, parse_summary(&value)),
            Err(e) => (true, Some(e.message()), None),
        },
        Err(GhError::NoPr) => (true, None, None),
        Err(e) if e.unavailable() => (false, Some(e.message()), None),
        Err(e) => (true, Some(e.message()), None),
    };
    StatusResponse {
        available,
        error,
        branch,
        pr,
    }
}

#[derive(Serialize)]
struct ListRow {
    #[serde(flatten)]
    summary: PrSummary,
    author: String,
    updated_at: String,
    repo: String,
}

fn repo_of_url(url: &str) -> String {
    let path = url.split("://").nth(1).unwrap_or(url);
    let mut parts = path.split('/');
    let _host = parts.next();
    match (parts.next(), parts.next()) {
        (Some(owner), Some(repo)) => format!("{owner}/{repo}"),
        _ => String::new(),
    }
}

async fn pr_list(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SessionQuery>,
) -> Response {
    let root = match session_repo_root(&state, &query.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    let wanted = match query.state.as_deref().unwrap_or("open") {
        s @ ("open" | "closed" | "merged" | "all") => s.to_string(),
        _ => return err_response(axum::http::StatusCode::BAD_REQUEST, "unknown state filter"),
    };
    let fields = format!("{SUMMARY_FIELDS},author,updatedAt");
    let gh = Gh::system();
    let raw = match gh
        .run(
            &root,
            &[
                "pr", "list", "--state", &wanted, "--limit", "50", "--json", &fields,
            ],
            None,
        )
        .await
    {
        Ok(raw) => raw,
        Err(e) if e.unavailable() => {
            return json_response(&json!({"available": false, "error": e.message(), "prs": []}))
        }
        Err(e) => return fail(&e),
    };
    let value = match parse_json(&raw) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    let rows: Vec<ListRow> = value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let summary = parse_summary(item)?;
            let author = item
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let updated_at = item
                .get("updatedAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let repo = repo_of_url(&summary.url);
            Some(ListRow {
                summary,
                author,
                updated_at,
                repo,
            })
        })
        .collect();
    json_response(&json!({"available": true, "error": null, "prs": rows}))
}

async fn pr_detail(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SessionQuery>,
) -> Response {
    let root = match session_repo_root(&state, &query.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    let Some(number) = query.number else {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "number is required");
    };
    match detail_in(&Gh::system(), &root, number).await {
        Ok(value) => json_response(&value),
        Err(e) => fail(&e),
    }
}

async fn detail_in(gh: &Gh, root: &Path, number: u64) -> Result<Value, GhError> {
    let fields = format!("{SUMMARY_FIELDS},body,author,comments,commits,files");
    let raw = gh
        .run(
            root,
            &["pr", "view", &number.to_string(), "--json", &fields],
            None,
        )
        .await?;
    let value = parse_json(&raw)?;
    let summary = parse_summary(&value)
        .ok_or_else(|| GhError::Failed("gh returned no pull request".into()))?;
    let comments: Vec<Value> = value
        .get("comments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            json!({
                "author": c.pointer("/author/login").and_then(Value::as_str).unwrap_or_default(),
                "body": c.get("body").and_then(Value::as_str).unwrap_or_default(),
                "created_at": c.get("createdAt").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    let commits: Vec<Value> = value
        .get("commits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            json!({
                "sha": c.get("oid").and_then(Value::as_str).unwrap_or_default(),
                "headline": c.get("messageHeadline").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect();
    let files: Vec<Value> = value
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| {
            json!({
                "path": f.get("path").and_then(Value::as_str).unwrap_or_default(),
                "additions": f.get("additions").and_then(Value::as_u64).unwrap_or(0),
                "deletions": f.get("deletions").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect();
    Ok(json!({
        "pr": summary,
        "body": value.get("body").and_then(Value::as_str).unwrap_or_default(),
        "author": value.pointer("/author/login").and_then(Value::as_str).unwrap_or_default(),
        "comments": comments,
        "commits": commits,
        "files": files,
    }))
}

fn valid_sha(sha: &str) -> bool {
    (7..=40).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Fold each run of removed lines that equals the following run of added lines modulo whitespace
/// into context, so a re-indent shows as no change.
pub(crate) fn fold_whitespace_changes(patch: &str) -> String {
    fn squash(line: &str) -> String {
        line.chars().filter(|c| !c.is_whitespace()).collect()
    }
    let lines: Vec<&str> = patch.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let is_del = line.starts_with('-') && !line.starts_with("---");
        if !is_del {
            out.push(line.to_string());
            i += 1;
            continue;
        }
        let mut j = i;
        while j < lines.len() && lines[j].starts_with('-') && !lines[j].starts_with("---") {
            j += 1;
        }
        let mut k = j;
        while k < lines.len() && lines[k].starts_with('+') && !lines[k].starts_with("+++") {
            k += 1;
        }
        let dels = &lines[i..j];
        let adds = &lines[j..k];
        let same = dels.len() == adds.len()
            && dels
                .iter()
                .zip(adds)
                .all(|(d, a)| squash(&d[1..]) == squash(&a[1..]));
        if same {
            out.extend(adds.iter().map(|a| format!(" {}", &a[1..])));
        } else {
            out.extend(lines[i..k].iter().map(|l| l.to_string()));
        }
        i = k;
    }
    out.join("\n")
}

async fn pr_diff(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<SessionQuery>,
) -> Response {
    let root = match session_repo_root(&state, &query.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    let Some(number) = query.number else {
        return err_response(axum::http::StatusCode::BAD_REQUEST, "number is required");
    };
    let hide = query.ignore_whitespace.unwrap_or(false);
    match diff_in(&Gh::system(), &root, number, query.commit.as_deref(), hide).await {
        Ok(value) => json_response(&value),
        Err(e) => fail(&e),
    }
}

async fn diff_in(
    gh: &Gh,
    root: &Path,
    number: u64,
    commit: Option<&str>,
    hide_whitespace: bool,
) -> Result<Value, GhError> {
    let number = number.to_string();
    let raw = match commit {
        Some(sha) => {
            if !valid_sha(sha) {
                return Err(GhError::Failed("commit must be a hex sha".into()));
            }
            let endpoint = format!("repos/{{owner}}/{{repo}}/commits/{sha}");
            gh.run(
                root,
                &[
                    "api",
                    "-H",
                    "Accept: application/vnd.github.diff",
                    &endpoint,
                ],
                None,
            )
            .await?
        }
        None => {
            gh.run(root, &["pr", "diff", &number, "--color", "never"], None)
                .await?
        }
    };
    let patch = if hide_whitespace {
        fold_whitespace_changes(&raw)
    } else {
        raw
    };
    Ok(json!({
        "files": parse_patch(&patch, hide_whitespace),
        "whitespace_hidden": hide_whitespace,
        "commit": commit,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    session: String,
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    draft: bool,
}

async fn pr_create(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<CreateRequest>,
) -> Response {
    let root = match session_repo_root(&state, &request.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    let title = request.title.trim();
    if title.is_empty() || title.contains('\n') {
        return err_response(
            axum::http::StatusCode::BAD_REQUEST,
            "a one-line title is required",
        );
    }
    let args = json!({
        "title": title,
        "body": request.body,
        "base": request.base,
        "draft": request.draft,
        "publish_images": false,
    });
    match forge_tools::create_pr_in(&root, &args).await {
        Ok(output) => {
            let url = output
                .split_whitespace()
                .map(|w| w.trim_matches(|c: char| matches!(c, '`' | '(' | ')' | ',' | '.')))
                .find(|w| w.starts_with("https://") || w.starts_with("http://"))
                .map(str::to_string);
            json_response(&json!({"ok": true, "url": url, "output": output}))
        }
        Err(message) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentRequest {
    session: String,
    number: u64,
    body: String,
}

async fn pr_comment(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<CommentRequest>,
) -> Response {
    let root = match session_repo_root(&state, &request.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    match comment_in(&Gh::system(), &root, request.number, &request.body).await {
        Ok(()) => json_response(&json!({"ok": true})),
        Err(e) => fail(&e),
    }
}

async fn comment_in(gh: &Gh, root: &Path, number: u64, body: &str) -> Result<(), GhError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(GhError::Failed("a comment cannot be empty".into()));
    }
    if body.len() > MAX_COMMENT_BYTES {
        return Err(GhError::Failed("the comment is too long".into()));
    }
    gh.run(
        root,
        &["pr", "comment", &number.to_string(), "--body-file", "-"],
        Some(body),
    )
    .await
    .map(|_| ())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeRequest {
    pub(crate) session: String,
    pub(crate) number: u64,
    pub(crate) head_sha: String,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) delete_branch: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct MergeResponse {
    ok: bool,
    /// `pulled` | `fetched` | `skipped`
    sync: &'static str,
    base: String,
}

async fn pr_merge(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<MergeRequest>,
) -> Response {
    let root = match session_repo_root(&state, &request.session).await {
        Ok(root) => root,
        Err(e) => return http_fail(e),
    };
    match merge_in(&Gh::system(), &root, &request).await {
        Ok(response) => json_response(&response),
        Err(e) => fail(&e),
    }
}

pub(crate) async fn merge_in(
    gh: &Gh,
    root: &Path,
    request: &MergeRequest,
) -> Result<MergeResponse, GhError> {
    if !valid_sha(&request.head_sha) {
        return Err(GhError::Failed(
            "head_sha is required: the commit the reviewer saw".into(),
        ));
    }
    let flag = match request.method.as_deref().unwrap_or("squash") {
        "squash" => "--squash",
        "merge" => "--merge",
        "rebase" => "--rebase",
        _ => {
            return Err(GhError::Failed(
                "method must be squash, merge or rebase".into(),
            ))
        }
    };
    let number = request.number.to_string();
    let base = gh
        .run(
            root,
            &["pr", "view", &number, "--json", "baseRefName"],
            None,
        )
        .await
        .ok()
        .and_then(|raw| parse_json(&raw).ok())
        .and_then(|v| {
            v.get("baseRefName")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let mut args = vec![
        "pr",
        "merge",
        &number,
        flag,
        "--match-head-commit",
        &request.head_sha,
    ];
    if request.delete_branch {
        args.push("--delete-branch");
    }
    gh.run(root, &args, None).await?;
    let sync = sync_base_after_merge(root, &base).await;
    Ok(MergeResponse {
        ok: true,
        sync,
        base,
    })
}

/// After a merge, bring the local base branch up to date: fast-forward when it is checked out and
/// clean, otherwise just fetch so the branch list shows it behind.
async fn sync_base_after_merge(root: &Path, base: &str) -> &'static str {
    if base.is_empty() {
        return "skipped";
    }
    let root = root.to_path_buf();
    let base = base.to_string();
    tokio::task::spawn_blocking(move || {
        let current = git_stdout(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
        let clean = git_stdout(&root, &["status", "--porcelain"])
            .map(|s| s.is_empty())
            .unwrap_or(false);
        let on_base = current.as_deref() == Some(base.as_str());
        let network = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
                .stdin(Stdio::null())
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        if on_base && clean {
            if network(&["pull", "--ff-only", "--quiet"]) {
                "pulled"
            } else {
                "skipped"
            }
        } else if network(&["fetch", "--quiet", "origin", &base]) {
            "fetched"
        } else {
            "skipped"
        }
    })
    .await
    .unwrap_or("skipped")
}

mod gh;
mod model;
#[cfg(test)]
mod tests;
