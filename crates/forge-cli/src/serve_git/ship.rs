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
    /// Plan only: answer with the files the commit would hold and change nothing.
    #[serde(default)]
    pub(crate) preview: bool,
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

#[derive(serde::Serialize, Clone, Debug)]
pub(crate) struct PlannedFile {
    pub(crate) path: String,
    /// "created" | "modified" | "deleted" | "renamed"
    pub(crate) kind: String,
}

/// What the commit step will hold, decided before anything is staged.
#[derive(serde::Serialize, Clone, Debug)]
pub(crate) struct CommitPlan {
    /// `index` (already staged, committed as it is) | `worktree` | `session` (this session's
    /// share of a shared checkout)
    pub(crate) source: &'static str,
    pub(crate) files: Vec<PlannedFile>,
    /// Dirty files in the tree that stay out of the commit.
    pub(crate) left_out: Vec<String>,
}

#[derive(serde::Serialize, Default)]
pub(crate) struct ShipResponse {
    pub(crate) ok: bool,
    pub(crate) preview: bool,
    pub(crate) plan: Option<CommitPlan>,
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

struct Pending {
    path: String,
    kind: &'static str,
}

fn kind_of(x: u8, y: u8) -> &'static str {
    match (x, y) {
        (b'?', b'?') | (b'A', _) | (_, b'A') => "created",
        (b'D', _) | (_, b'D') => "deleted",
        (b'R', _) | (_, b'R') => "renamed",
        _ => "modified",
    }
}

/// Dirty paths other than Forge's own state and untracked build residue. `Err` for unresolved
/// conflicts, which must not be committed.
fn pending_paths(root: &Path) -> Result<Vec<Pending>, String> {
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
        paths.push(Pending {
            path: path.to_string(),
            kind: kind_of(x, y),
        });
    }
    Ok(paths)
}

/// Files already in the index, against HEAD.
fn staged_files(root: &Path) -> Result<Vec<PlannedFile>, String> {
    let raw = git_raw(root, &["diff", "--cached", "--name-status", "-z"])?;
    let text = String::from_utf8_lossy(&raw);
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else { break };
        let kind = match status.as_bytes()[0] {
            b'A' => "created",
            b'D' => "deleted",
            b'R' | b'C' => "renamed",
            _ => "modified",
        };
        // A rename lists the old path first and the new one second.
        let path = if matches!(status.as_bytes()[0], b'R' | b'C') {
            fields.next().unwrap_or(path)
        } else {
            path
        };
        files.push(PlannedFile {
            path: path.to_string(),
            kind: kind.to_string(),
        });
    }
    Ok(files)
}

fn looks_like_secret(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name == ".env" || name.starts_with(".env.") {
        return !matches!(
            name.rsplit('.').next(),
            Some("example" | "sample" | "template" | "dist")
        );
    }
    name.ends_with(".pem")
        || name.ends_with(".key")
        || ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"]
            .iter()
            .any(|prefix| name.starts_with(prefix) && !name.ends_with(".pub"))
}

/// How the session's checkout is shared, which decides whose files are the session's.
pub(super) enum Scope {
    /// The session has its own worktree: everything dirty in it is the session's.
    Worktree,
    /// A checkout other sessions may edit too; `live` is every session the daemon knows.
    Shared {
        live: Vec<crate::serve_git::changes::LiveSession>,
    },
}

/// Decide what the commit holds: the index as it is when anything is staged, else the session's
/// own dirty files. Refuses what cannot be separated cleanly and files that look like secrets.
fn plan_commit(
    store: &forge_store::Store,
    root: &Path,
    session: &str,
    scope: Scope,
) -> Result<CommitPlan, String> {
    let pending = pending_paths(root)?;
    let staged = staged_files(root)?;
    if !staged.is_empty() {
        let in_index: std::collections::BTreeSet<&str> =
            staged.iter().map(|f| f.path.as_str()).collect();
        let left_out = pending
            .iter()
            .filter(|p| !in_index.contains(p.path.as_str()))
            .map(|p| p.path.clone())
            .collect();
        return Ok(CommitPlan {
            source: "index",
            files: staged,
            left_out,
        });
    }
    let (source, files, left_out) = match scope {
        Scope::Worktree => (
            "worktree",
            pending
                .iter()
                .map(|p| PlannedFile {
                    path: p.path.clone(),
                    kind: p.kind.to_string(),
                })
                .collect::<Vec<_>>(),
            Vec::new(),
        ),
        Scope::Shared { live } => {
            let split = super::changes::split_shared_tree(store, root, session, live);
            if !split.contested.is_empty() {
                return Err(format!(
                    "{} also changed by another session, so this session's files cannot be \
                     separated: {}. Stage what to ship in the Review dock and ship again",
                    if split.contested.len() == 1 {
                        "a file was"
                    } else {
                        "files were"
                    },
                    split.contested.join(", ")
                ));
            }
            if split.truncated {
                return Err(
                    "this session changed too many files to list; stage what to ship \
                            in the Review dock and ship again"
                        .to_string(),
                );
            }
            let known: std::collections::BTreeSet<&str> =
                pending.iter().map(|p| p.path.as_str()).collect();
            let files = split
                .mine
                .into_iter()
                .filter(|(path, _)| known.contains(path.as_str()))
                .map(|(path, kind)| PlannedFile { path, kind })
                .collect::<Vec<_>>();
            (
                "session",
                files,
                split
                    .others
                    .into_iter()
                    .filter(|p| known.contains(p.as_str()))
                    .collect(),
            )
        }
    };
    let secrets: Vec<&str> = files
        .iter()
        .filter(|f| looks_like_secret(&f.path) && super::diff::is_untracked(root, &f.path))
        .map(|f| f.path.as_str())
        .collect();
    if !secrets.is_empty() {
        return Err(format!(
            "untracked files that look like secrets: {}. Add them to .gitignore, move them out \
             of the repository, or stage them yourself if they are meant to be committed",
            secrets.join(", ")
        ));
    }
    Ok(CommitPlan {
        source,
        files,
        left_out,
    })
}

/// Stage the planned files into an index that holds nothing yet.
fn stage_plan(root: &Path, plan: &CommitPlan) -> Result<(), String> {
    if plan.source == "index" || plan.files.is_empty() {
        return Ok(());
    }
    let mut args = vec!["add", "-A", "--"];
    args.extend(plan.files.iter().map(|f| f.path.as_str()));
    git_raw(root, &args).map(|_| ())
}

/// Put the index back to what it was before [`stage_plan`]: empty against HEAD.
fn unstage_plan(root: &Path, plan: &CommitPlan) {
    if plan.source == "index" || plan.files.is_empty() {
        return;
    }
    let mut args = vec!["reset", "-q", "--"];
    args.extend(plan.files.iter().map(|f| f.path.as_str()));
    if git_raw(root, &args).is_err() {
        let mut args = vec!["rm", "-r", "-q", "--cached", "--ignore-unmatch", "--"];
        args.extend(plan.files.iter().map(|f| f.path.as_str()));
        let _ = git_raw(root, &args);
    }
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

async fn unstage(root: &Path, plan: &CommitPlan) {
    let (root, plan) = (root.to_path_buf(), plan.clone());
    let _ = tokio::task::spawn_blocking(move || unstage_plan(&root, &plan)).await;
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
    let live = super::changes::live_sessions(state).await;

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

    let worktree = {
        let store = state.store.clone();
        let id = session.to_string();
        let hosted = state
            .registry
            .get(session)
            .await
            .and_then(|handle| handle.worktree.clone());
        match hosted {
            Some(worktree) => Some(worktree),
            None => tokio::task::spawn_blocking(move || store.session_worktree(&id).ok().flatten())
                .await
                .ok()
                .flatten(),
        }
    };
    let scope = match worktree {
        Some(worktree) if Path::new(&worktree).is_dir() => Scope::Worktree,
        _ => Scope::Shared { live },
    };
    let plan_root = root.clone();
    let plan_store = state.store.clone();
    let plan_session = session.to_string();
    let planned = tokio::task::spawn_blocking(move || {
        plan_commit(&plan_store, &plan_root, &plan_session, scope)
    })
    .await
    .map_err(|_| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git failed".to_string(),
        )
    })?;
    let plan = match planned {
        Ok(plan) => plan,
        Err(reason) => return Ok(out.refuse("commit", reason)),
    };
    out.plan = Some(plan.clone());
    if request.preview {
        out.preview = true;
        out.skipped(
            "commit",
            format!("preview: {} file(s) would be committed", plan.files.len()),
        );
        out.ok = true;
        return Ok(out);
    }

    if plan.files.is_empty() {
        out.skipped(
            "commit",
            "nothing to commit; shipping the commits already on the branch",
        );
    } else {
        let stage_root = root.clone();
        let stage_plan_copy = plan.clone();
        let staged = tokio::task::spawn_blocking(move || stage_plan(&stage_root, &stage_plan_copy))
            .await
            .map_err(|_| {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "git failed".to_string(),
                )
            })?;
        if let Err(reason) = staged {
            unstage(&root, &plan).await;
            return Ok(out.refuse("commit", reason));
        }
        let message = match request
            .message
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            Some(message) => message.to_string(),
            None => match super::message::generate(state, session, &root, None).await {
                Ok(generated) => generated.message,
                Err(reason) => {
                    unstage(&root, &plan).await;
                    return Ok(out.refuse("commit", reason));
                }
            },
        };
        let commit_root = root.clone();
        let commit_message = message.clone();
        let committed = tokio::task::spawn_blocking(move || {
            git_raw(
                &commit_root,
                &["commit", "-m", commit_message.as_str(), "--"],
            )?;
            Ok::<_, String>(git_stdout(&commit_root, &["rev-parse", "HEAD"]).unwrap_or_default())
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
            Err(reason) => {
                unstage(&root, &plan).await;
                return Ok(out.refuse("commit", reason));
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
