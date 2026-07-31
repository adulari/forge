use super::*;

struct SessionGitContext {
    dir: String,
    managed_worktree: bool,
    busy: bool,
}

async fn session_git_context(state: &DaemonState, session: &str) -> Option<SessionGitContext> {
    let handle = state.registry.get(session).await?;
    let managed_worktree = handle.worktree.is_some();
    let dir = handle
        .worktree
        .clone()
        .unwrap_or_else(|| handle.cwd.clone());
    let self_busy = handle.snapshot_rx.borrow().snapshot.busy;
    // A shared checkout is process-global repository state: switching it underneath a different
    // busy session is just as unsafe as switching underneath the addressed one. Worktree-backed
    // sessions are isolated and do not participate in this check.
    let busy = if managed_worktree {
        self_busy
    } else {
        let busy_shared_dirs = state
            .registry
            .all()
            .await
            .into_iter()
            .filter(|peer| peer.worktree.is_none() && peer.snapshot_rx.borrow().snapshot.busy)
            .map(|peer| peer.cwd.clone())
            .collect::<Vec<_>>();
        let target_dir = dir.clone();
        tokio::task::spawn_blocking(move || {
            let Ok(target_root) = repo_root_of(&target_dir) else {
                return self_busy;
            };
            busy_shared_dirs
                .iter()
                .filter_map(|peer_dir| repo_root_of(peer_dir).ok())
                .any(|peer_root| peer_root == target_root)
        })
        .await
        .unwrap_or(self_busy)
    };
    Some(SessionGitContext {
        dir,
        managed_worktree,
        busy,
    })
}

pub(super) fn is_forge_managed_worktree(path: &str) -> bool {
    let components: Vec<_> = Path::new(path).components().collect();
    components
        .windows(2)
        .any(|window| window[0].as_os_str() == ".forge" && window[1].as_os_str() == "worktrees")
}

pub(super) fn parse_worktree_branches(raw: &str) -> std::collections::HashMap<String, String> {
    let mut branches = std::collections::HashMap::new();
    let mut path: Option<String> = None;
    for line in raw.lines().chain(std::iter::once("")) {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
            if let Some(worktree) = path.as_ref() {
                branches.insert(value.to_string(), worktree.clone());
            }
        } else if line.is_empty() {
            path = None;
        }
    }
    branches
}

fn worktree_state(
    root: &Path,
) -> Result<(std::collections::HashMap<String, String>, bool), String> {
    let raw =
        String::from_utf8_lossy(&git_raw(root, &["worktree", "list", "--porcelain"])?).into_owned();
    let branches = parse_worktree_branches(&raw);
    let has_managed = branches
        .values()
        .any(|path| is_forge_managed_worktree(path));
    Ok((branches, has_managed))
}

fn default_branch(root: &Path) -> Option<String> {
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
    .and_then(|name| name.strip_prefix("origin/").map(str::to_string))
}

pub(super) fn parse_branch_rows(
    raw: &str,
    current: Option<&str>,
    default: Option<&str>,
    worktrees: &std::collections::HashMap<String, String>,
) -> Vec<GitBranchRow> {
    let mut rows = Vec::new();
    for record in raw.lines() {
        let mut fields = record.split('\0');
        let Some(full_name) = fields.next() else {
            continue;
        };
        let oid = fields.next().unwrap_or_default().to_string();
        let upstream = fields
            .next()
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let (name, remote) = if let Some(name) = full_name.strip_prefix("refs/heads/") {
            (name.to_string(), false)
        } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
            // `origin/HEAD` is a symbolic convenience ref, not a checkout target.
            if name.ends_with("/HEAD") {
                continue;
            }
            (name.to_string(), true)
        } else {
            continue;
        };
        let local_default = default.is_some_and(|candidate| !remote && candidate == name);
        let remote_default = default.is_some_and(|candidate| {
            remote
                && name
                    .strip_prefix("origin/")
                    .is_some_and(|tail| tail == candidate)
        });
        rows.push(GitBranchRow {
            current: !remote && current.is_some_and(|candidate| candidate == name),
            default: local_default || remote_default,
            worktree: (!remote).then(|| worktrees.get(&name).cloned()).flatten(),
            name,
            oid,
            upstream,
            remote,
        });
    }
    rows.sort_by(|a, b| {
        (
            !a.current,
            a.remote,
            !a.default,
            a.worktree.is_none(),
            a.name.to_lowercase(),
        )
            .cmp(&(
                !b.current,
                b.remote,
                !b.default,
                b.worktree.is_none(),
                b.name.to_lowercase(),
            ))
    });
    rows
}

fn branch_action_blocker(
    root: &Path,
    managed_worktree: bool,
    busy: bool,
    has_managed_worktrees: bool,
) -> Result<Option<String>, String> {
    if managed_worktree {
        return Ok(Some(
            "This isolated Forge worktree keeps its generated branch until you merge or discard the session."
                .to_string(),
        ));
    }
    if busy {
        return Ok(Some(
            "Wait for active turns using this shared repository to finish before changing branches."
                .to_string(),
        ));
    }
    if has_managed_worktrees {
        return Ok(Some(
            "Merge or discard Forge worktree sessions before changing the shared workspace branch."
                .to_string(),
        ));
    }
    if !git_raw(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Ok(Some(
            "Commit, stash, or discard working tree changes before changing branches.".to_string(),
        ));
    }
    Ok(None)
}

fn branches_impl(
    context: SessionGitContext,
    query: String,
    limit: usize,
) -> Result<GitBranchesResponse, String> {
    let root = repo_root_of(&context.dir)?;
    let current = git_stdout(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    let default = default_branch(&root);
    let (worktrees, has_managed_worktrees) = worktree_state(&root)?;
    let raw = String::from_utf8_lossy(&git_raw(
        &root,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname:short)%00%(upstream:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )?)
    .into_owned();
    let query = query.trim().to_lowercase();
    let mut rows = parse_branch_rows(&raw, current.as_deref(), default.as_deref(), &worktrees);
    if !query.is_empty() {
        rows.retain(|row| row.name.to_lowercase().contains(&query));
    }
    let limit = limit.clamp(1, MAX_BRANCH_ROWS);
    let truncated = rows.len().saturating_sub(limit);
    rows.truncate(limit);
    let actions_blocked_reason = branch_action_blocker(
        &root,
        context.managed_worktree,
        context.busy,
        has_managed_worktrees,
    )?;
    Ok(GitBranchesResponse {
        root: root.to_string_lossy().into_owned(),
        current,
        default_branch: default,
        managed_worktree: context.managed_worktree,
        actions_blocked_reason,
        branches: rows,
        truncated,
    })
}

/// `GET /api/git/branches?session=<id>&q=<query>&limit=<n>`.
pub(crate) async fn git_branches(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<GitBranchesQuery>,
) -> Response {
    let Some(context) = session_git_context(&state, &params.session).await else {
        return no_such_session();
    };
    match tokio::task::spawn_blocking(move || branches_impl(context, params.q, params.limit)).await
    {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git branch listing failed",
        ),
    }
}

fn valid_branch_name(root: &Path, raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("a branch name is required".to_string());
    }
    git_raw(root, &["check-ref-format", "--branch", name])
        .map_err(|_| format!("invalid branch name: {name}"))?;
    Ok(name.to_string())
}

fn ref_exists(root: &Path, full_ref: &str) -> bool {
    std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "show-ref",
            "--verify",
            "--quiet",
            full_ref,
        ])
        .status()
        .is_ok_and(|status| status.success())
}

type BranchActionResult = Result<GitBranchActionResponse, (axum::http::StatusCode, String)>;

fn branch_action_ready(
    context: &SessionGitContext,
) -> Result<PathBuf, (axum::http::StatusCode, String)> {
    let root = repo_root_of(&context.dir)
        .map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?;
    let (_, has_managed_worktrees) =
        worktree_state(&root).map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?;
    if let Some(reason) = branch_action_blocker(
        &root,
        context.managed_worktree,
        context.busy,
        has_managed_worktrees,
    )
    .map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?
    {
        return Err((axum::http::StatusCode::CONFLICT, reason));
    }
    Ok(root)
}

fn switch_branch_impl(context: SessionGitContext, raw_branch: String) -> BranchActionResult {
    let root = branch_action_ready(&context)?;
    let branch = valid_branch_name(&root, &raw_branch)
        .map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?;
    let current = git_stdout(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    if current.as_deref() == Some(branch.as_str()) {
        return Ok(GitBranchActionResponse { ok: true, branch });
    }

    let local_ref = format!("refs/heads/{branch}");
    if ref_exists(&root, &local_ref) {
        let (worktrees, _) = worktree_state(&root)
            .map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?;
        if let Some(path) = worktrees.get(&branch) {
            return Err((
                axum::http::StatusCode::CONFLICT,
                format!("branch {branch} is checked out in worktree {path}"),
            ));
        }
        git_raw(&root, &["switch", branch.as_str()])
            .map_err(|message| (axum::http::StatusCode::CONFLICT, message))?;
    } else {
        let remote_ref = format!("refs/remotes/{branch}");
        if !ref_exists(&root, &remote_ref) {
            return Err((
                axum::http::StatusCode::NOT_FOUND,
                format!("no such local or remote branch: {branch}"),
            ));
        }
        let Some((_, local_name)) = branch.split_once('/') else {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                "remote branch must include its remote name".to_string(),
            ));
        };
        let local_name = valid_branch_name(&root, local_name)
            .map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?;
        if ref_exists(&root, &format!("refs/heads/{local_name}")) {
            return Err((
                axum::http::StatusCode::CONFLICT,
                format!("local branch {local_name} already exists; select it instead"),
            ));
        }
        git_raw(
            &root,
            &[
                "switch",
                "--track",
                "-c",
                local_name.as_str(),
                branch.as_str(),
            ],
        )
        .map_err(|message| (axum::http::StatusCode::CONFLICT, message))?;
    }
    let branch = git_stdout(&root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|message| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, message))?;
    Ok(GitBranchActionResponse { ok: true, branch })
}

fn create_branch_impl(context: SessionGitContext, raw_name: String) -> BranchActionResult {
    let root = branch_action_ready(&context)?;
    let name = valid_branch_name(&root, &raw_name)
        .map_err(|message| (axum::http::StatusCode::BAD_REQUEST, message))?;
    if ref_exists(&root, &format!("refs/heads/{name}")) {
        return Err((
            axum::http::StatusCode::CONFLICT,
            format!("branch already exists: {name}"),
        ));
    }
    git_raw(&root, &["switch", "-c", name.as_str()])
        .map_err(|message| (axum::http::StatusCode::CONFLICT, message))?;
    Ok(GitBranchActionResponse {
        ok: true,
        branch: name,
    })
}

fn branch_action_response(result: Result<BranchActionResult, tokio::task::JoinError>) -> Response {
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err((status, message))) => err_response(status, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git branch action failed",
        ),
    }
}

/// `POST /api/git/switch` `{session, branch}`.
pub(crate) async fn git_switch_branch(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitSwitchRequest>,
) -> Response {
    let Some(context) = session_git_context(&state, &request.session).await else {
        return no_such_session();
    };
    branch_action_response(
        tokio::task::spawn_blocking(move || switch_branch_impl(context, request.branch)).await,
    )
}

/// `POST /api/git/branches` `{session, name}` — create and switch.
pub(crate) async fn git_create_branch(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<GitCreateBranchRequest>,
) -> Response {
    let Some(context) = session_git_context(&state, &request.session).await else {
        return no_such_session();
    };
    branch_action_response(
        tokio::task::spawn_blocking(move || create_branch_impl(context, request.name)).await,
    )
}
