//! What a host looks like from a session's workspace: machine identity plus the git layout around
//! the workspace. Read straight from `.git` files (no `git` subprocess) so it is cheap enough to
//! capture on every session start and resume.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_WORKTREES: usize = 32;
const MAX_SIBLINGS: usize = 64;

/// A git checkout and its remotes, canonicalized (`host/owner/repo`, no credentials).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoRef {
    pub path: String,
    #[serde(default)]
    pub remotes: Vec<String>,
}

/// The host a session was last grounded on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostFingerprint {
    pub host: String,
    /// Hash prefix of `/etc/machine-id`; survives hostname changes, which laptops do often.
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub home: Option<String>,
    pub cwd: String,
    #[serde(default)]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    /// The workspace repo, its git worktrees and the git checkouts beside it.
    #[serde(default)]
    pub repos: Vec<RepoRef>,
}

impl HostFingerprint {
    pub fn capture(cwd: &Path) -> Self {
        let root = repo_root(cwd);
        let (branch, head) = root.as_deref().map(head_of).unwrap_or_default();
        let repos = root.as_deref().map(layout_repos).unwrap_or_default();
        Self {
            host: hostname(),
            machine_id: machine_id(),
            home: std::env::var("HOME").ok().filter(|h| !h.is_empty()),
            cwd: cwd.display().to_string(),
            repo_root: root.map(|r| r.display().to_string()),
            branch,
            head,
            repos,
        }
    }

    pub fn label(&self) -> String {
        format!("host `{}` (workspace {})", self.host, self.cwd)
    }

    /// Why `self` (recorded) and `now` are different places, or `None` when they are the same.
    pub fn moved_reason(&self, now: &Self) -> Option<String> {
        let different_machine = match (&self.machine_id, &now.machine_id) {
            (Some(a), Some(b)) => a != b,
            _ => self.host != now.host,
        };
        if different_machine {
            return Some(format!(
                "it was last active on host `{}` and now runs on host `{}`",
                self.host, now.host
            ));
        }
        (self.cwd != now.cwd).then(|| {
            format!(
                "its workspace moved from {} to {} on host `{}`",
                self.cwd, now.cwd, now.host
            )
        })
    }
}

/// The workspace repo, every worktree of it, and the git checkouts in its parent directory.
fn layout_repos(root: &Path) -> Vec<RepoRef> {
    let mut paths = vec![root.to_path_buf()];
    paths.extend(worktrees(root));
    paths.extend(sibling_dirs(root).into_iter().filter(|d| is_repo(d)));
    let mut seen = std::collections::BTreeSet::new();
    paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .map(|p| RepoRef {
            remotes: remotes(&p),
            path: p.display().to_string(),
        })
        .collect()
}

pub(crate) fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

pub(crate) fn repo_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|d| is_repo(d))
        .map(Path::to_path_buf)
}

fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let target = Path::new(text.trim().strip_prefix("gitdir:")?.trim());
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    })
}

fn common_dir(git_dir: &Path) -> PathBuf {
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => {
            let target = Path::new(text.trim());
            let joined = if target.is_absolute() {
                target.to_path_buf()
            } else {
                git_dir.join(target)
            };
            joined.canonicalize().unwrap_or(joined)
        }
        Err(_) => git_dir.to_path_buf(),
    }
}

/// Canonical remote identities from the repo's (shared) git config.
pub(crate) fn remotes(root: &Path) -> Vec<String> {
    let Some(git) = git_dir(root) else {
        return Vec::new();
    };
    let Ok(config) = std::fs::read_to_string(common_dir(&git).join("config")) else {
        return Vec::new();
    };
    let mut in_remote = false;
    let mut out = Vec::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_remote = line.starts_with("[remote ");
            continue;
        }
        if !in_remote {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "url" && !value.trim().is_empty() {
                out.push(crate::capsule::canonical_remote_identity(value.trim()));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `(branch, commit)` of the checkout's HEAD.
pub(crate) fn head_of(root: &Path) -> (Option<String>, Option<String>) {
    let Some(git) = git_dir(root) else {
        return (None, None);
    };
    let Ok(head) = std::fs::read_to_string(git.join("HEAD")) else {
        return (None, None);
    };
    let head = head.trim();
    let Some(reference) = head.strip_prefix("ref: ") else {
        return (None, Some(head.to_string()));
    };
    let branch = reference.strip_prefix("refs/heads/").map(str::to_string);
    let common = common_dir(&git);
    let commit = [git.join(reference), common.join(reference)]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|c| c.trim().to_string())
        .or_else(|| {
            let packed = std::fs::read_to_string(common.join("packed-refs")).ok()?;
            packed.lines().find_map(|line| {
                let (hash, name) = line.split_once(' ')?;
                (name == reference).then(|| hash.to_string())
            })
        });
    (branch, commit)
}

/// The main checkout plus every linked worktree that still exists.
pub(crate) fn worktrees(root: &Path) -> Vec<PathBuf> {
    let Some(git) = git_dir(root) else {
        return Vec::new();
    };
    let common = common_dir(&git);
    let mut out = Vec::new();
    if common.file_name().is_some_and(|n| n == ".git") {
        if let Some(main) = common.parent() {
            out.push(main.to_path_buf());
        }
    }
    let mut linked: Vec<PathBuf> = std::fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("gitdir")).ok())
        .filter_map(|g| Path::new(g.trim()).parent().map(Path::to_path_buf))
        .filter(|p| p.is_dir())
        .collect();
    linked.sort();
    out.extend(linked);
    out.truncate(MAX_WORKTREES);
    out
}

/// Non-hidden directories next to `root`, sorted.
pub(crate) fn sibling_dirs(root: &Path) -> Vec<PathBuf> {
    let Some(parent) = root.parent() else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p != root)
        .collect();
    dirs.sort();
    dirs.truncate(MAX_SIBLINGS);
    dirs
}

pub(crate) fn hostname() -> String {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()))
        .or_else(|| std::env::var("COMPUTERNAME").ok().filter(|h| !h.is_empty()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .map(|id| hex::encode(&Sha256::digest(id.as_bytes())[..8]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_move::test_support::fake_repo;

    #[test]
    fn capture_reads_branch_head_remotes_and_sibling_checkouts() {
        let tmp = tempfile::tempdir().unwrap();
        let api = tmp.path().canonicalize().unwrap().join("API");
        fake_repo(&api.join("app"), "git@github.com:Acme/App.git");
        fake_repo(&api.join("dash"), "https://github.com/acme/dash");
        std::fs::create_dir_all(api.join("notes")).unwrap();
        let fp = HostFingerprint::capture(&api.join("app"));
        assert_eq!(fp.branch.as_deref(), Some("main"));
        assert_eq!(fp.head.as_deref(), Some("abc123"));
        assert_eq!(fp.repos[0].remotes, vec!["github.com/acme/app"]);
        let paths: Vec<_> = fp.repos.iter().map(|r| r.path.clone()).collect();
        assert!(paths.iter().any(|p| p.ends_with("dash")));
        assert!(!paths.iter().any(|p| p.ends_with("notes")));
    }

    #[test]
    fn moved_reason_prefers_machine_id_over_hostname() {
        let base = HostFingerprint {
            host: "laptop".into(),
            machine_id: Some("m1".into()),
            cwd: "/w".into(),
            ..Default::default()
        };
        let renamed = HostFingerprint {
            host: "laptop.local".into(),
            ..base.clone()
        };
        assert_eq!(base.moved_reason(&renamed), None);
        let other = HostFingerprint {
            host: "server".into(),
            machine_id: Some("m2".into()),
            ..base.clone()
        };
        assert!(base.moved_reason(&other).unwrap().contains("`server`"));
        let moved_cwd = HostFingerprint {
            cwd: "/elsewhere".into(),
            ..base.clone()
        };
        assert!(base
            .moved_reason(&moved_cwd)
            .unwrap()
            .contains("workspace moved"));
    }
}
