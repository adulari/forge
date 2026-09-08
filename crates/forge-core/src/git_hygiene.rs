//! Commit discipline: keep the model committing the work it does.
//!
//! A model left alone in a repository edits for hours and never commits. Measured on one session
//! before this existed: six days, 2,600 successful edits, 49 `git commit` calls, 2 pushes, two
//! whole days at zero commits, 48 dirty files at the end. Nothing in the harness ever mentioned
//! the working tree to the model, so nothing ever prompted the commit.
//!
//! Two reminders, both cheap and both about the files THIS session touched (a user's own unrelated
//! dirty files are never nagged about):
//!
//! - **turn start** — while any file this session edited is still uncommitted, the turn's context
//!   carries one line naming them; if the branch is also well ahead of its upstream, the same line
//!   asks the model to offer the user a push. Bundled with the prompt, so it costs no extra call.
//! - **mid-turn** — after every `commit_nudge_edits` successful edits since the last commit or
//!   reminder, one system hint lands right after the tool result. Also no extra call: the loop
//!   was going to continue anyway.
//!
//! Forge never commits or pushes by itself here; the model does, through the same permission
//! broker as every other shell command, and pushing is always the user's call.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What `git` says about the working tree and branch right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RepoStatus {
    /// Paths (relative to the repo root) with uncommitted changes, tracked or untracked.
    pub dirty: BTreeSet<PathBuf>,
    /// Commits the current branch is ahead of its upstream; `None` when there is no upstream.
    pub ahead: Option<u32>,
    pub branch: Option<String>,
}

impl RepoStatus {
    /// Parse `git status --porcelain=v1 --branch` output.
    pub(crate) fn parse(text: &str) -> Self {
        let mut status = Self::default();
        for line in text.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                let (name, rest) = header.split_once(' ').unwrap_or((header, ""));
                let branch = name.split("...").next().unwrap_or(name);
                if !branch.starts_with("HEAD (") && !branch.starts_with("No commits") {
                    status.branch = Some(branch.to_string());
                }
                if name.contains("...") {
                    status.ahead = Some(
                        rest.split(|c: char| !c.is_ascii_alphanumeric())
                            .collect::<Vec<_>>()
                            .windows(2)
                            .find(|w| w[0] == "ahead")
                            .and_then(|w| w[1].parse().ok())
                            .unwrap_or(0),
                    );
                }
                continue;
            }
            if line.len() < 4 {
                continue;
            }
            let path = &line[3..];
            // A rename shows as `R  old -> new`; the new name is the one on disk.
            let path = path.rsplit(" -> ").next().unwrap_or(path);
            let path = path.trim_matches('"');
            if !path.is_empty() {
                status.dirty.insert(PathBuf::from(path));
            }
        }
        status
    }
}

/// Run `git status` in `root`. `None` outside a repository or without git.
pub(crate) fn probe(root: &Path) -> Option<RepoStatus> {
    let out = std::process::Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=normal",
        ])
        .current_dir(root)
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    Some(RepoStatus::parse(&String::from_utf8_lossy(&out.stdout)))
}

/// The repository root may be a parent of the workspace root (a crate inside a monorepo), and
/// `git status` reports paths relative to it.
fn toplevel(root: &Path) -> PathBuf {
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
        .unwrap_or_else(|| root.to_path_buf())
}

/// Per-session bookkeeping: which files the session wrote, how many edits since the last commit
/// or reminder, and which branch head the push reminder already covered.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    touched: BTreeSet<PathBuf>,
    edits_since_nudge: u32,
    head: Option<String>,
    push_nudged_head: Option<String>,
    top: Option<PathBuf>,
}

/// A reminder the session should place in front of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Nudge {
    /// Uncommitted files this session edited.
    Commit {
        files: Vec<PathBuf>,
        ahead: Option<u32>,
    },
    /// Nothing of ours is dirty, but the branch has commits the remote does not.
    Push { ahead: u32, branch: Option<String> },
}

impl Tracker {
    #[cfg(test)]
    pub(crate) fn touched_count(&self) -> usize {
        self.touched.len()
    }

    /// Record one successful write-tool call on `path`. Returns `true` when the mid-turn reminder
    /// is due (the counter resets so it is due again after another `every` edits).
    pub(crate) fn record_edit(&mut self, root: &Path, path: &Path, every: u32) -> bool {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let top = self.top.get_or_insert_with(|| toplevel(root));
        if let Ok(rel) = abs.strip_prefix(top) {
            self.touched.insert(rel.to_path_buf());
        }
        self.edits_since_nudge += 1;
        if every > 0 && self.edits_since_nudge >= every {
            self.edits_since_nudge = 0;
            return true;
        }
        false
    }

    /// Called after a shell command that mentions git ran: if HEAD moved, a commit happened, and
    /// everything that is no longer dirty is forgotten.
    pub(crate) fn observe_head(&mut self, root: &Path, head: Option<String>) -> bool {
        if head == self.head {
            return false;
        }
        self.head = head;
        self.edits_since_nudge = 0;
        self.retain_dirty(root);
        true
    }

    /// Drop touched files that `git status` no longer lists as dirty (committed, reverted, or
    /// rewound). Returns the current status so the caller can reuse it.
    pub(crate) fn retain_dirty(&mut self, root: &Path) -> Option<RepoStatus> {
        let status = probe(root)?;
        self.touched.retain(|p| status.dirty.contains(p));
        Some(status)
    }

    /// The turn-start reminder, if any. Re-probes the tree so a commit made outside Forge (or a
    /// rewind that restored the files) silences it. `None` when nothing of ours is dirty and the
    /// branch is not far enough ahead — or when the push reminder already covered this head.
    pub(crate) fn turn_start_nudge(
        &mut self,
        root: &Path,
        head: Option<String>,
        push_ahead: u32,
    ) -> Option<Nudge> {
        self.head = head;
        let status = self.retain_dirty(root)?;
        if !self.touched.is_empty() {
            return Some(Nudge::Commit {
                files: self.touched.iter().cloned().collect(),
                ahead: status.ahead.filter(|&n| push_ahead > 0 && n >= push_ahead),
            });
        }
        let ahead = status.ahead?;
        if push_ahead == 0 || ahead < push_ahead || self.push_nudged_head == self.head {
            return None;
        }
        self.push_nudged_head = self.head.clone();
        Some(Nudge::Push {
            ahead,
            branch: status.branch,
        })
    }

    /// The mid-turn reminder text, for the files still dirty right now.
    pub(crate) fn mid_turn_nudge(&mut self, root: &Path) -> Option<Nudge> {
        self.retain_dirty(root)?;
        if self.touched.is_empty() {
            return None;
        }
        Some(Nudge::Commit {
            files: self.touched.iter().cloned().collect(),
            ahead: None,
        })
    }
}

/// Whether a shell command could have moved HEAD (commit, amend, reset, merge, rebase, …).
pub(crate) fn may_move_head(command: &str) -> bool {
    command.contains("git ") || command.contains("gh ") || command.contains("jj ")
}

impl Nudge {
    pub(crate) fn render(&self) -> String {
        match self {
            Nudge::Commit { files, ahead } => {
                let listed: Vec<String> = files
                    .iter()
                    .take(8)
                    .map(|p| p.display().to_string())
                    .collect();
                let more = files.len().saturating_sub(listed.len());
                let names = if more > 0 {
                    format!("{}, … and {more} more", listed.join(", "))
                } else {
                    listed.join(", ")
                };
                let mut text = format!(
                    "[git] {} file{} you edited in this session {} still uncommitted: {names}. \
                     Commit each verified unit of work as you finish it — stage those specific \
                     files (never `git add -A`) with a focused conventional-commit message — and \
                     leave any unrelated changes the user has in the tree alone. If the work is \
                     not ready to commit, say so in your reply.",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" },
                    if files.len() == 1 { "is" } else { "are" },
                );
                if let Some(n) = ahead {
                    text.push_str(&format!(
                        " The branch is also {n} commit{} ahead of its upstream: after committing, \
                         ask the user (ask_user) whether to push — do not push unasked.",
                        if *n == 1 { "" } else { "s" }
                    ));
                }
                text
            }
            Nudge::Push { ahead, branch } => format!(
                "[git] {}{ahead} commit{} ahead of its upstream and nothing is uncommitted. Ask \
                 the user (ask_user) whether to push now — do not push unasked.",
                branch
                    .as_deref()
                    .map(|b| format!("branch `{b}` is "))
                    .unwrap_or_else(|| "the branch is ".to_string()),
                if *ahead == 1 { "" } else { "s" }
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        git(dir.path(), &["add", "a.rs"]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn parses_branch_ahead_and_dirty_paths() {
        let s = RepoStatus::parse(
            "## main...origin/main [ahead 9, behind 1]\n M src/lib.rs\n?? new.txt\nR  old.rs -> new.rs\n",
        );
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.ahead, Some(9));
        let dirty: Vec<_> = s.dirty.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(dirty, ["new.rs", "new.txt", "src/lib.rs"]);
    }

    #[test]
    fn no_upstream_means_no_ahead_count() {
        let s = RepoStatus::parse("## feature\n M x\n");
        assert_eq!(s.ahead, None);
        assert_eq!(s.branch.as_deref(), Some("feature"));
    }

    #[test]
    fn the_mid_turn_reminder_is_due_every_n_edits_and_only_for_still_dirty_files() {
        let dir = repo();
        let root = dir.path();
        let mut t = Tracker::default();
        std::fs::write(root.join("a.rs"), "fn a() { 1 }\n").unwrap();
        assert!(!t.record_edit(root, &root.join("a.rs"), 3));
        assert!(!t.record_edit(root, &root.join("a.rs"), 3));
        assert!(
            t.record_edit(root, &root.join("a.rs"), 3),
            "third edit is due"
        );
        let nudge = t.mid_turn_nudge(root).unwrap();
        assert!(matches!(&nudge, Nudge::Commit { files, .. } if files == &[PathBuf::from("a.rs")]));
        assert!(nudge.render().contains("a.rs"));
        // A commit (HEAD moved) clears the file.
        git(root, &["commit", "-q", "-am", "work"]);
        let head = Some(git(root, &["rev-parse", "HEAD"]));
        assert!(t.observe_head(root, head));
        assert_eq!(t.touched_count(), 0);
        assert!(t.mid_turn_nudge(root).is_none());
    }

    #[test]
    fn an_unrelated_dirty_file_is_never_mentioned() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("users-own.txt"), "x").unwrap();
        let mut t = Tracker::default();
        assert!(t.turn_start_nudge(root, None, 3).is_none());
        std::fs::write(root.join("a.rs"), "fn a() { 2 }\n").unwrap();
        t.record_edit(root, Path::new("a.rs"), 0);
        match t.turn_start_nudge(root, None, 3) {
            Some(Nudge::Commit { files, ahead }) => {
                assert_eq!(files, [PathBuf::from("a.rs")]);
                assert_eq!(ahead, None, "no upstream → no push talk");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_disabled_mid_turn_counter_never_fires() {
        let dir = repo();
        let mut t = Tracker::default();
        for _ in 0..50 {
            assert!(!t.record_edit(dir.path(), Path::new("a.rs"), 0));
        }
    }

    #[test]
    fn the_push_reminder_fires_once_per_head_when_the_branch_is_ahead() {
        let dir = repo();
        let root = dir.path();
        let remote = tempfile::tempdir().unwrap();
        git(remote.path(), &["init", "-q", "--bare"]);
        git(
            root,
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(root, &["push", "-q", "-u", "origin", "main"]);
        for i in 0..3 {
            std::fs::write(root.join("a.rs"), format!("fn a() {{ {i} }}\n")).unwrap();
            git(root, &["commit", "-q", "-am", "more"]);
        }
        let head = Some(git(root, &["rev-parse", "HEAD"]));
        let mut t = Tracker::default();
        let nudge = t.turn_start_nudge(root, head.clone(), 3).unwrap();
        assert!(matches!(nudge, Nudge::Push { ahead: 3, .. }));
        assert!(nudge.render().contains("Ask the user"));
        assert!(
            t.turn_start_nudge(root, head, 3).is_none(),
            "same head → silent"
        );
        let mut strict = Tracker::default();
        assert!(
            strict.turn_start_nudge(root, None, 4).is_none(),
            "below threshold"
        );
        let mut off = Tracker::default();
        assert!(off.turn_start_nudge(root, None, 0).is_none(), "0 disables");
    }

    #[test]
    fn render_caps_the_file_list() {
        let files: Vec<PathBuf> = (0..12).map(|i| PathBuf::from(format!("f{i}.rs"))).collect();
        let text = Nudge::Commit {
            files,
            ahead: Some(5),
        }
        .render();
        assert!(text.contains("… and 4 more"));
        assert!(text.contains("5 commits ahead"));
        assert!(text.contains("do not push unasked"));
    }
}
