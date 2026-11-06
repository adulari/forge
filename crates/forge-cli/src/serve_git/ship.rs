//! `POST /api/sessions/{id}/ship` — a finished session to a watched pull request in one call.
//!
//! The steps run in order and stop at the first refusal, which comes back as data with the step
//! that refused and why (HTTP 200, `ok: false`), so every surface can show the same progress rows:
//!
//! 1. `branch`  refuses a busy session, a detached HEAD and the base branch itself
//! 2. `commit`  stages the session's changes and commits them with a generated (or given) message
//! 3. `push`    pushes the branch, never forced (done by `open_pr`'s own checks and push)
//! 4. `pr`      opens the PR with a generated (or given) title and body plus the session's provenance
//! 5. `watch`   arms PR Watch in merge mode; the watcher then pushes merged / blocked
//!
//! The commit, push and PR steps reuse the dock's commit-message generation and the `open_pr`
//! checks; nothing here talks to git or `gh` in a way those do not already.

use std::path::Path;

use super::message::trivial_call;
use super::*;
use crate::serve_pr_watch::{arm_watch, Mode, Watch};

const PR_SYSTEM: &str = "You write pull request text. Reply with a one-line title first \
(conventional-commit style, imperative mood, at most 72 characters, no trailing period), then a \
blank line, then a short Markdown body: two to six bullets saying what changed and why. Base it \
strictly on the commit list and diff stat; do not invent changes. No code fences, no preamble.";

const MAX_TITLE_CHARS: usize = 120;
const MAX_LOG_CHARS: usize = 6_000;

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShipRequest {
    /// Commit message; generated from the staged diff when absent.
    #[serde(default)]
    pub(crate) message: Option<String>,
    /// PR title; generated when absent. A one-line override of what the sheet previewed.
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) body: Option<String>,
    #[serde(default)]
    pub(crate) base: Option<String>,
    #[serde(default)]
    pub(crate) draft: bool,
    /// `squash` (default) | `merge` | `rebase` | `auto`.
    #[serde(default)]
    pub(crate) method: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct Step {
    step: &'static str,
    /// `done` | `skipped` | `refused`
    status: &'static str,
    detail: String,
}

#[derive(serde::Serialize)]
pub(crate) struct Refusal {
    step: &'static str,
    reason: String,
}

#[derive(serde::Serialize)]
pub(crate) struct ShipCommit {
    sha: String,
    message: String,
}

#[derive(serde::Serialize)]
pub(crate) struct ShipPr {
    number: Option<u64>,
    url: String,
    title: String,
    /// The branch already had an open PR; the push updated it and title/body were left alone.
    existing: bool,
}

#[derive(serde::Serialize, Default)]
pub(crate) struct ShipResponse {
    pub(crate) ok: bool,
    pub(crate) steps: Vec<Step>,
    pub(crate) branch: Option<String>,
    pub(crate) base: Option<String>,
    pub(crate) commit: Option<ShipCommit>,
    pub(crate) pr: Option<ShipPr>,
    pub(crate) watch: Option<Watch>,
    pub(crate) refused: Option<Refusal>,
}

impl ShipResponse {
    fn done(&mut self, step: &'static str, detail: impl Into<String>) {
        self.steps.push(Step {
            step,
            status: "done",
            detail: detail.into(),
        });
    }

    fn skipped(&mut self, step: &'static str, detail: impl Into<String>) {
        self.steps.push(Step {
            step,
            status: "skipped",
            detail: detail.into(),
        });
    }

    fn refuse(mut self, step: &'static str, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        self.steps.push(Step {
            step,
            status: "refused",
            detail: reason.clone(),
        });
        self.refused = Some(Refusal { step, reason });
        self
    }
}

/// The base branch to compare against: the request's, else the remote's default.
fn default_base(root: &Path, requested: Option<&str>) -> Option<String> {
    if let Some(base) = requested.map(str::trim).filter(|b| !b.is_empty()) {
        return Some(base.to_string());
    }
    git_stdout(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .ok()
    .and_then(|r| r.strip_prefix("origin/").map(str::to_string))
    .filter(|b| !b.is_empty())
}

/// Paths to stage: everything the session changed except Forge's own state and untracked build
/// residue. `Err` for unresolved conflicts, which must not be committed.
fn pending_paths(root: &Path) -> Result<Vec<String>, String> {
    let raw = git_raw(root, &["status", "--porcelain=v1", "-z", "-uall"])?;
    let text = String::from_utf8_lossy(&raw);
    let mut fields = text.split('\0');
    let mut paths = Vec::new();
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (x, y) = (entry.as_bytes()[0], entry.as_bytes()[1]);
        let path = &entry[3..];
        if matches!(x, b'R' | b'C') {
            fields.next();
        }
        if x == b'U' || y == b'U' || (x == b'A' && y == b'A') || (x == b'D' && y == b'D') {
            return Err(format!("{path} has unresolved merge conflicts"));
        }
        if path.starts_with(".forge/") || (x == b'?' && is_noise_path(path)) {
            continue;
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn stage_and_check(root: &Path) -> Result<bool, String> {
    let paths = pending_paths(root)?;
    if !paths.is_empty() {
        let mut args = vec!["add", "-A", "--"];
        args.extend(paths.iter().map(String::as_str));
        git_raw(root, &args)?;
    }
    Ok(!git_stdout(root, &["diff", "--cached", "--name-only"])
        .unwrap_or_default()
        .is_empty())
}

fn clip_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// A title and body from the commits being shipped, with no model.
fn commit_log_text(subjects: &[String]) -> (String, String) {
    let title = subjects
        .first()
        .cloned()
        .unwrap_or_else(|| "chore: update files".to_string());
    let body = subjects
        .iter()
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n");
    (title, body)
}

/// Split a model reply into a one-line title and the rest as body.
fn split_title_body(reply: &str) -> Option<(String, String)> {
    let mut lines = reply.lines().skip_while(|l| l.trim().is_empty());
    let first = lines.next()?;
    let title = first
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_start_matches("Title:")
        .trim()
        .trim_matches('"');
    if title.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.trim();
    let body = body.strip_prefix("Body:").unwrap_or(body).trim();
    Some((clip_chars(title, MAX_TITLE_CHARS), body.to_string()))
}

async fn pr_text(
    state: &DaemonState,
    session: &str,
    root: &Path,
    base: Option<&str>,
) -> (String, String) {
    let root_owned = root.to_path_buf();
    let base_owned = base.map(str::to_string);
    let (subjects, stat) = tokio::task::spawn_blocking(move || {
        let range = base_owned
            .as_deref()
            .map(|b| format!("refs/remotes/origin/{b}"))
            .filter(|r| git_stdout(&root_owned, &["rev-parse", "--verify", "-q", r]).is_ok())
            .map(|r| format!("{r}..HEAD"));
        let log_args: Vec<&str> = match &range {
            Some(r) => vec!["log", "--format=%s", r.as_str()],
            None => vec!["log", "-n", "5", "--format=%s"],
        };
        let subjects: Vec<String> = git_stdout(&root_owned, &log_args)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        let stat = range
            .as_deref()
            .and_then(|r| {
                git_stdout(
                    &root_owned,
                    &["diff", "--stat=100", "--no-color", &r.replace("..", "...")],
                )
                .ok()
            })
            .unwrap_or_default();
        (subjects, stat)
    })
    .await
    .unwrap_or_default();
    let fallback = commit_log_text(&subjects);
    if state.mock || subjects.is_empty() {
        return fallback;
    }
    let prompt = format!(
        "Commits on this branch (newest first):\n{}\n\nDiff stat:\n{}",
        clip_chars(&subjects.join("\n"), MAX_LOG_CHARS),
        clip_chars(&stat, MAX_LOG_CHARS)
    );
    match trivial_call(
        state,
        session,
        "pr-text",
        "a pull request title and description",
        PR_SYSTEM,
        prompt,
    )
    .await
    {
        Ok((reply, _)) => split_title_body(&reply).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

fn pr_url_and_number(output: &str) -> (String, Option<u64>) {
    let url = output
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, '`' | '(' | ')' | ',' | '.')))
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .unwrap_or_default()
        .to_string();
    let number = url.rsplit('/').next().and_then(|n| n.parse().ok());
    (url, number)
}

/// Run the ship steps for `session`. `Err` only for a request that could not start (unknown
/// session, bad method); a refusal is an `Ok` response with `ok: false`.
pub(crate) async fn ship_session(
    state: &Arc<DaemonState>,
    session: &str,
    request: &ShipRequest,
    gh: &Path,
) -> Result<ShipResponse, (axum::http::StatusCode, String)> {
    let method = request.method.as_deref().unwrap_or("squash");
    if !matches!(method, "squash" | "merge" | "rebase" | "auto") {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "method must be squash, merge, rebase or auto".into(),
        ));
    }
    let root = crate::serve_git::session_repo_root(state, session).await?;
    let mut out = ShipResponse::default();

    if let Some(handle) = state.registry.get(session).await {
        if handle.snapshot_rx.borrow().snapshot.busy {
            return Ok(out.refuse(
                "branch",
                "the session is still working; ship when it finishes",
            ));
        }
    }

    let probe_root = root.clone();
    let requested_base = request.base.clone();
    let (branch, base) = tokio::task::spawn_blocking(move || {
        let branch = git_stdout(&probe_root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()
            .filter(|b| !b.is_empty());
        let base = default_base(&probe_root, requested_base.as_deref());
        (branch, base)
    })
    .await
    .map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git failed".to_string(),
        )
    })?;
    out.branch = branch.clone();
    out.base = base.clone();
    let Some(branch) = branch else {
        return Ok(out.refuse(
            "branch",
            "HEAD is detached; create a branch first (git switch -c <name>)",
        ));
    };
    let on_base = match &base {
        Some(base) => *base == branch,
        None => matches!(branch.as_str(), "main" | "master"),
    };
    if on_base {
        return Ok(out.refuse(
            "branch",
            format!("on `{branch}`, the base branch; create a branch first"),
        ));
    }
    out.done("branch", format!("shipping `{branch}`"));

    let stage_root = root.clone();
    let staged = tokio::task::spawn_blocking(move || stage_and_check(&stage_root))
        .await
        .map_err(|_| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "git failed".to_string(),
            )
        })?;
    match staged {
        Err(reason) => return Ok(out.refuse("commit", reason)),
        Ok(false) => out.skipped(
            "commit",
            "nothing to commit; shipping the commits already on the branch",
        ),
        Ok(true) => {
            let message = match request
                .message
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
            {
                Some(message) => message.to_string(),
                None => match super::message::generate(state, session, &root, None).await {
                    Ok(generated) => generated.message,
                    Err(reason) => return Ok(out.refuse("commit", reason)),
                },
            };
            let commit_root = root.clone();
            let commit_message = message.clone();
            let committed = tokio::task::spawn_blocking(move || {
                git_raw(
                    &commit_root,
                    &["commit", "-m", commit_message.as_str(), "--"],
                )?;
                Ok::<_, String>(
                    git_stdout(&commit_root, &["rev-parse", "HEAD"]).unwrap_or_default(),
                )
            })
            .await
            .map_err(|_| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "git commit failed".to_string(),
                )
            })?;
            match committed {
                Ok(sha) => {
                    let subject = message.lines().next().unwrap_or_default().to_string();
                    out.done("commit", format!("{} {subject}", &sha[..sha.len().min(7)]));
                    out.commit = Some(ShipCommit { sha, message });
                }
                Err(reason) => return Ok(out.refuse("commit", reason)),
            }
        }
    }

    let (generated_title, generated_body) = pr_text(state, session, &root, base.as_deref()).await;
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map_or(generated_title, |t| clip_chars(t, MAX_TITLE_CHARS));
    if title.contains('\n') {
        return Ok(out.refuse("pr", "the title must be one line"));
    }
    let body = request.body.clone().unwrap_or(generated_body);
    let args = serde_json::json!({
        "title": title,
        "body": body,
        "base": request.base,
        "draft": request.draft,
        "allow_untracked": true,
        "publish_images": false,
    });
    let output = match forge_tools::create_pr_for_session(&root, &args, session, gh).await {
        Ok(output) => output,
        Err(reason) => {
            let step = if reason.starts_with("push refused") {
                "push"
            } else {
                "pr"
            };
            return Ok(out.refuse(step, reason));
        }
    };
    let existing = output.starts_with("pushed ");
    let (url, number) = pr_url_and_number(&output);
    out.done("push", format!("pushed `{branch}`"));
    let label = number.map_or_else(|| "PR".to_string(), |n| format!("PR #{n}"));
    out.done(
        "pr",
        if existing {
            format!("{label} already open; updated by the push")
        } else {
            format!("{label} opened: {title}")
        },
    );
    out.pr = Some(ShipPr {
        number,
        url,
        title,
        existing,
    });

    out.watch = arm_watch(state, session, Mode::Merge, method).await;
    // The watcher acts only on sessions this daemon hosts; a session that lives elsewhere (a
    // terminal chat) is armed all the same and starts being watched once it runs here.
    let hosted = state.registry.get(session).await.is_some();
    let note = if hosted {
        ""
    } else {
        "; the session is not running on this daemon, so watching starts when it is resumed here"
    };
    out.done(
        "watch",
        format!("watching {label} in merge mode ({method}){note}"),
    );
    out.ok = true;
    Ok(out)
}

/// `POST /api/sessions/{id}/ship` `{message?, title?, body?, base?, draft?, method?}`.
pub(crate) async fn session_ship(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<ShipRequest>,
) -> Response {
    match ship_session(&state, &id, &request, Path::new("gh")).await {
        Ok(response) => json_response(&response),
        Err((status, message)) => err_response(status, &message),
    }
}

#[cfg(test)]
mod tests;
