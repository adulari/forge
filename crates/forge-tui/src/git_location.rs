//! Where the chat is working: the repository and branch, read straight from `.git`.
//!
//! The statusline used to learn the branch once, at startup, by spawning `git`, and nothing updated
//! it afterwards, so a checkout during the session (the agent's or the user's) left it naming a
//! branch the workspace had already left. Its repository name was the top-level directory's
//! basename, which inside a linked worktree is the worktree's own directory (`dev`, `pr-1377`)
//! rather than the repository. Reading the few files git itself reads costs no process, so it can
//! be repeated every couple of seconds, and it says which repository a worktree belongs to.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::App;

const REFRESH_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHead {
    Branch(String),
    /// A checked-out commit rather than a branch: the first seven hex digits.
    Detached(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLocation {
    pub repo: String,
    pub head: GitHead,
    /// A linked worktree (`git worktree add`), not the repository's main checkout.
    pub worktree: bool,
}

impl GitLocation {
    pub fn branch_label(&self) -> String {
        match &self.head {
            GitHead::Branch(name) => name.clone(),
            GitHead::Detached(sha) => format!("@{sha}"),
        }
    }
}

/// The repository and branch `start` is inside, or `None` outside a git checkout.
pub fn read_git_location(start: &Path) -> Option<GitLocation> {
    let (dot_git, top) = start.ancestors().find_map(|dir| {
        let candidate = dir.join(".git");
        candidate.exists().then(|| (candidate, dir.to_path_buf()))
    })?;
    // A `.git` FILE points somewhere else: a linked worktree's private directory, or a submodule's
    // directory inside its parent's `.git/modules`.
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if target.is_absolute() {
            target
        } else {
            top.join(target)
        }
    };
    // Only a linked worktree has `commondir`; it leads to the repository's shared `.git`.
    let commondir = std::fs::read_to_string(git_dir.join("commondir")).ok();
    let worktree = commondir.is_some();
    let common = match commondir {
        Some(rel) => {
            let rel = PathBuf::from(rel.trim());
            let joined = if rel.is_absolute() {
                rel
            } else {
                git_dir.join(rel)
            };
            std::fs::canonicalize(&joined).unwrap_or(joined)
        }
        None => git_dir.clone(),
    };
    let head = parse_head(&std::fs::read_to_string(git_dir.join("HEAD")).ok()?)?;
    Some(GitLocation {
        repo: repo_name(&common, &top)?,
        head,
        worktree,
    })
}

fn repo_name(common: &Path, top: &Path) -> Option<String> {
    let base = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    match common.file_name().and_then(|n| n.to_str()) {
        // The ordinary layout: `<repo>/.git`.
        Some(".git") => common.parent().and_then(base),
        // A submodule's directory sits in its parent's `.git/modules/…`; the checkout names it.
        _ if common.ancestors().any(|a| a.ends_with("modules")) => base(top),
        // A bare repository: `forge.git`.
        Some(name) => Some(name.trim_end_matches(".git").to_string()),
        None => base(top),
    }
}

fn parse_head(text: &str) -> Option<GitHead> {
    let text = text.trim();
    if let Some(reference) = text.strip_prefix("ref:") {
        let reference = reference.trim();
        let branch = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return Some(GitHead::Branch(branch.to_string()));
    }
    (text.len() >= 7 && text.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| GitHead::Detached(text[..7].to_string()))
}

impl App {
    /// Follow `workspace`'s repository and branch in the statusline, reading them now.
    pub fn watch_git_location(&mut self, workspace: PathBuf) {
        self.git_workspace = Some(workspace);
        self.git_checked_at = None;
        self.refresh_git_location(Instant::now());
    }

    /// Re-read the location at most every two seconds. True when what the statusline shows changed.
    pub fn refresh_git_location(&mut self, now: Instant) -> bool {
        let Some(workspace) = &self.git_workspace else {
            return false;
        };
        if self
            .git_checked_at
            .is_some_and(|at| now.duration_since(at) < REFRESH_EVERY)
        {
            return false;
        }
        self.git_checked_at = Some(now);
        let location = read_git_location(workspace);
        // Outside a checkout the `RepoName` widget still names the directory, as it always has.
        let repo_name = location.as_ref().map(|l| l.repo.clone()).or_else(|| {
            workspace
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        });
        let changed = location != self.git_location;
        self.git_branch = location.as_ref().map(GitLocation::branch_label);
        self.repo_name = repo_name;
        self.git_location = location;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(name: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "forge-git-location-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let dir = dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn an_ordinary_checkout_names_its_directory_and_branch_from_any_subdirectory() {
        let repo = scratch("forge");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/main\n");
        let nested = repo.join("crates/forge-tui");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            read_git_location(&nested),
            Some(GitLocation {
                repo: "forge".into(),
                head: GitHead::Branch("main".into()),
                worktree: false,
            })
        );
    }

    #[test]
    fn a_linked_worktree_names_the_repository_it_belongs_to_not_its_own_directory() {
        let repo = scratch("forge");
        let private = repo.join(".git/worktrees/dev");
        write(
            &private.join("HEAD"),
            "ref: refs/heads/feat/tool-output-viewer\n",
        );
        write(&private.join("commondir"), "../..\n");
        let checkout = repo.parent().unwrap().join("dev");
        write(
            &checkout.join(".git"),
            &format!("gitdir: {}\n", private.display()),
        );
        let location = read_git_location(&checkout).unwrap();
        assert_eq!(location.repo, "forge");
        assert_eq!(
            location.head,
            GitHead::Branch("feat/tool-output-viewer".into())
        );
        assert!(location.worktree);
    }

    #[test]
    fn a_checked_out_commit_is_shown_as_a_short_hash() {
        let repo = scratch("forge");
        write(
            &repo.join(".git/HEAD"),
            "e50b52d7a1b2c3d4e5f60718293a4b5c6d7e8f90\n",
        );
        let location = read_git_location(&repo).unwrap();
        assert_eq!(location.head, GitHead::Detached("e50b52d".into()));
        assert_eq!(location.branch_label(), "@e50b52d");
    }

    #[test]
    fn a_checkout_during_the_session_reaches_the_statusline() {
        let repo = scratch("forge");
        write(&repo.join(".git/HEAD"), "ref: refs/heads/main\n");
        let mut app = App::default();
        app.watch_git_location(repo.clone());
        assert_eq!(app.git_branch.as_deref(), Some("main"));

        write(&repo.join(".git/HEAD"), "ref: refs/heads/feat/x\n");
        let start = app.git_checked_at.unwrap();
        assert!(
            !app.refresh_git_location(start + Duration::from_millis(500)),
            "rate-limited: no re-read inside the window"
        );
        assert!(app.refresh_git_location(start + REFRESH_EVERY));
        assert_eq!(app.git_branch.as_deref(), Some("feat/x"));
        assert_eq!(app.repo_name.as_deref(), Some("forge"));
    }

    #[test]
    fn outside_a_checkout_there_is_no_location() {
        assert_eq!(parse_head("not a head"), None);
        assert_eq!(read_git_location(&scratch("plain")), None);
    }
}
