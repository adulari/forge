//! `GET /api/git/status` — the review dock's file list, parsed from `git status --porcelain=v1 -z`
//! and the two `--numstat` listings.

use super::*;

/// Parse `git status --porcelain=v1 -z`: `XY <path>\0`, with an extra NUL-separated ORIGIN path
/// following a rename/copy record. NUL framing (not the default line framing) so paths containing
/// spaces, quotes, or newlines survive verbatim.
pub(super) fn parse_status_z(raw: &str) -> Vec<(char, char, String, Option<String>)> {
    let mut out = Vec::new();
    let mut fields = raw.split('\0');
    while let Some(record) = fields.next() {
        if record.len() < 4 {
            continue;
        }
        let mut chars = record.chars();
        let (Some(x), Some(y)) = (chars.next(), chars.next()) else {
            continue;
        };
        let path = record[3..].to_string();
        let orig = if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
            fields.next().map(str::to_string)
        } else {
            None
        };
        out.push((x, y, path, orig));
    }
    out
}

/// Parse `git diff --numstat -z` into `path → (adds, dels, binary)`.
///
/// Normal record: `adds\tdels\tpath\0`. Rename record: `adds\tdels\t\0old\0new\0` — the path field
/// is empty and the two names follow, so the fields must be walked in order rather than split
/// independently. Binary files report `-` for both counts.
pub(super) fn parse_numstat_z(
    raw: &str,
) -> std::collections::HashMap<String, (usize, usize, bool)> {
    let mut out = std::collections::HashMap::new();
    let mut fields = raw.split('\0');
    while let Some(record) = fields.next() {
        let record = record.trim_start_matches('\n');
        if record.trim().is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let (Some(adds), Some(dels), Some(rest)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let binary = adds == "-" || dels == "-";
        let adds = adds.parse::<usize>().unwrap_or(0);
        let dels = dels.parse::<usize>().unwrap_or(0);
        let path = if rest.is_empty() {
            // Rename/copy: `\0old\0new\0` — the destination name is what status reports.
            let _old = fields.next();
            match fields.next() {
                Some(new) => new.to_string(),
                None => continue,
            }
        } else {
            rest.to_string()
        };
        out.insert(path, (adds, dels, binary));
    }
    out
}

/// Porcelain `XY` codes that mean an unresolved merge conflict: any `U`, plus both-added and
/// both-deleted.
fn is_conflict(x: char, y: char) -> bool {
    x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D')
}

/// The unfinished operation that explains conflicted files, read from git's own state files.
fn in_progress_operation(root: &Path) -> Option<String> {
    [
        ("merge", "MERGE_HEAD"),
        ("rebase", "rebase-merge"),
        ("rebase", "rebase-apply"),
        ("cherry-pick", "CHERRY_PICK_HEAD"),
        ("revert", "REVERT_HEAD"),
    ]
    .into_iter()
    .find_map(|(name, state)| {
        let path = git_stdout(root, &["rev-parse", "--git-path", state]).ok()?;
        root.join(path).exists().then(|| name.to_string())
    })
}

fn status_rows(
    entries: &[(char, char, String, Option<String>)],
    numstat: &std::collections::HashMap<String, (usize, usize, bool)>,
    pick: impl Fn(char, char) -> Option<char>,
    truncated: &mut usize,
) -> Vec<GitFileRow> {
    let mut rows = Vec::new();
    for (x, y, path, orig) in entries {
        let Some(letter) = pick(*x, *y) else { continue };
        if rows.len() >= MAX_STATUS_ROWS {
            *truncated += 1;
            continue;
        }
        let (adds, dels, binary) = numstat.get(path).copied().unwrap_or((0, 0, false));
        rows.push(GitFileRow {
            path: path.clone(),
            status: letter.to_string(),
            orig_path: orig.clone(),
            adds,
            dels,
            binary,
        });
    }
    rows
}

/// The status snapshot for the repository enclosing `dir`. `base_repo` is the base checkout when
/// `dir` is a forge worktree.
pub(super) fn status_response(
    dir: &str,
    base_repo: Option<PathBuf>,
) -> Result<GitStatusResponse, String> {
    let root = repo_root_of(dir)?;
    let base_branch = base_repo
        .and_then(|repo| git_stdout(&repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok());
    let status = String::from_utf8_lossy(&git_raw(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?)
    .into_owned();
    let entries = parse_status_z(&status);
    let unstaged_stat = parse_numstat_z(&String::from_utf8_lossy(&git_raw(
        &root,
        &["diff", "--numstat", "-z"],
    )?));
    let staged_stat = parse_numstat_z(&String::from_utf8_lossy(&git_raw(
        &root,
        &["diff", "--cached", "--numstat", "-z"],
    )?));
    let branch = git_stdout(&root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let mut truncated = 0usize;
    let staged = status_rows(
        &entries,
        &staged_stat,
        |x, y| (x != ' ' && x != '?' && !is_conflict(x, y)).then_some(x),
        &mut truncated,
    );
    let unstaged = status_rows(
        &entries,
        &unstaged_stat,
        |x, y| (y != ' ' && y != '?' && !is_conflict(x, y)).then_some(y),
        &mut truncated,
    );
    let conflicted = status_rows(
        &entries,
        &std::collections::HashMap::new(),
        |x, y| is_conflict(x, y).then_some('U'),
        &mut truncated,
    );
    // Untracked files have no numstat; count their lines so the row agrees with the diff
    // route (it used to say +0 -0 for a 20,000-line file the diff showed as all additions).
    let mut untracked_stat = std::collections::HashMap::new();
    let mut budget: u64 = 16 * 1024 * 1024;
    for (x, y, path, _) in &entries {
        if *x == '?' && *y == '?' {
            if let Some(counts) = untracked_counts(&root, path, &mut budget) {
                untracked_stat.insert(path.clone(), counts);
            }
        }
    }
    let untracked = status_rows(
        &entries,
        &untracked_stat,
        |x, y| (x == '?' && y == '?').then_some('?'),
        &mut truncated,
    );
    let upstream = git_stdout(
        &root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .ok()
    .filter(|name| !name.is_empty());
    let (ahead, behind) = upstream
        .as_ref()
        .and_then(|_| {
            git_stdout(
                &root,
                &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
            )
            .ok()
        })
        .and_then(|counts| {
            let (ahead, behind) = counts.split_once('\t')?;
            Some((ahead.trim().parse().ok()?, behind.trim().parse().ok()?))
        })
        .unwrap_or((0, 0));
    let has_remote = git_stdout(&root, &["remote"]).is_ok_and(|names| !names.is_empty());
    let operation = in_progress_operation(&root);
    Ok(GitStatusResponse {
        root: root.to_string_lossy().into_owned(),
        branch,
        base_branch,
        staged,
        unstaged,
        untracked,
        conflicted,
        truncated,
        upstream,
        ahead,
        behind,
        operation,
        has_remote,
    })
}

/// `GET /api/git/status?session=<id>` — the review dock's file list, bucketed exactly as git sees
/// it (a file edited after being staged legitimately appears in BOTH `staged` and `unstaged`).
pub(crate) async fn git_status(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<GitSessionQuery>,
) -> Response {
    let Some(dir) = session_dir(&state, &params.session).await else {
        return no_such_session();
    };
    let base_repo = worktree_repo_and_branch(&dir).map(|(repo, _)| repo);
    let result = tokio::task::spawn_blocking(move || status_response(&dir, base_repo)).await;
    match result {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(message)) => err_response(axum::http::StatusCode::BAD_REQUEST, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "git status failed",
        ),
    }
}
