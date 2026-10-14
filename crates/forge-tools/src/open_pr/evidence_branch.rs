//! Publishing screenshots to the orphan `forge-evidence` branch with git plumbing alone.
//!
//! GitHub has no API for attaching images to a PR, but a blob link at a fixed commit renders
//! inline for anyone who can read the repo. Objects are written with `hash-object`, the tree is
//! built in a private index file under the git dir, and the commit is pushed straight to
//! `refs/heads/forge-evidence`: the worktree, the real index and HEAD are never touched. Each
//! publish adds `<session>/<file>` on top of the branch's current tip, so evidence from other
//! sessions stays where it is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::command::{run, run_raw};
use super::github::Repo;

pub(super) const BRANCH: &str = "forge-evidence";
const REF: &str = "refs/heads/forge-evidence";
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Forge"),
    ("GIT_AUTHOR_EMAIL", "forge@adulari.dev"),
    ("GIT_COMMITTER_NAME", "Forge"),
    ("GIT_COMMITTER_EMAIL", "forge@adulari.dev"),
];

#[derive(Debug)]
pub(super) struct Published {
    pub commit: String,
    /// Image URL per evidence file name.
    pub urls: HashMap<String, String>,
}

/// Publish `files` (evidence file name, local path) under `<session>/` on the remote's
/// `forge-evidence` branch. One retry covers a concurrent publish moving the tip.
pub(super) async fn publish(
    git: &Path,
    repo_dir: &Path,
    remote: &str,
    repo: &Repo,
    session: &str,
    files: &[(String, PathBuf)],
) -> Result<Published, String> {
    let mut last_error = String::new();
    for _ in 0..2 {
        match publish_once(git, repo_dir, remote, session, files).await {
            Ok(commit) => {
                let urls = files
                    .iter()
                    .map(|(name, _)| {
                        (
                            name.clone(),
                            repo.raw_url(&commit, &format!("{session}/{name}")),
                        )
                    })
                    .collect();
                return Ok(Published { commit, urls });
            }
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

async fn publish_once(
    git: &Path,
    repo_dir: &Path,
    remote: &str,
    session: &str,
    files: &[(String, PathBuf)],
) -> Result<String, String> {
    let parent = remote_tip(git, repo_dir, remote).await?;
    let index = PathBuf::from(
        run(
            git,
            &["rev-parse", "--git-path", "forge-evidence.index"],
            repo_dir,
            &[],
        )
        .await?,
    );
    let index = if index.is_absolute() {
        index
    } else {
        repo_dir.join(index)
    };
    let _ = std::fs::remove_file(&index);
    let result = build_and_push(
        git,
        repo_dir,
        remote,
        session,
        files,
        parent.as_deref(),
        &index,
    )
    .await;
    let _ = std::fs::remove_file(&index);
    result
}

async fn build_and_push(
    git: &Path,
    repo_dir: &Path,
    remote: &str,
    session: &str,
    files: &[(String, PathBuf)],
    parent: Option<&str>,
    index: &Path,
) -> Result<String, String> {
    let index_str = index.display().to_string();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    if let Some(parent) = parent {
        run(git, &["read-tree", parent], repo_dir, &env).await?;
    }
    for (name, path) in files {
        let path = path.display().to_string();
        let blob = run(git, &["hash-object", "-w", "--", &path], repo_dir, &[]).await?;
        let entry = format!("100644,{blob},{session}/{name}");
        run(
            git,
            &["update-index", "--add", "--cacheinfo", &entry],
            repo_dir,
            &env,
        )
        .await?;
    }
    let tree = run(git, &["write-tree"], repo_dir, &env).await?;
    if let Some(parent) = parent {
        let parent_tree = run(
            git,
            &["rev-parse", &format!("{parent}^{{tree}}")],
            repo_dir,
            &[],
        )
        .await?;
        if parent_tree == tree {
            return Ok(parent.to_string());
        }
    }
    let message = format!("evidence: session {session}");
    let mut args = vec![
        "commit-tree",
        "--no-gpg-sign",
        tree.as_str(),
        "-m",
        message.as_str(),
    ];
    if let Some(parent) = parent {
        args.extend(["-p", parent]);
    }
    let commit = run(git, &args, repo_dir, &IDENTITY).await?;
    let refspec = format!("{commit}:{REF}");
    run(git, &["push", "--quiet", remote, &refspec], repo_dir, &[]).await?;
    Ok(commit)
}

/// The remote branch tip, fetched into the object store when it is not there yet. `None` when the
/// branch does not exist (the first publish creates it as an orphan).
async fn remote_tip(git: &Path, repo_dir: &Path, remote: &str) -> Result<Option<String>, String> {
    let listing = run(git, &["ls-remote", remote, REF], repo_dir, &[]).await?;
    let Some(tip) = listing
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let present = run_raw(
        git,
        &["cat-file", "-e", &format!("{tip}^{{commit}}")],
        repo_dir,
        &[],
        None,
    )
    .await?;
    if !present.ok {
        run(
            git,
            &["fetch", "--quiet", "--no-tags", remote, REF],
            repo_dir,
            &[],
        )
        .await?;
    }
    Ok(Some(tip))
}
