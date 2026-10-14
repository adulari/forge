//! What the `/goal` evaluator is shown as proof of work: commits since the goal started, the diff
//! against the goal's base, uncommitted files, and passing checks from the evidence ledger.
//!
//! Everything here is read from git and the ledger, never from the model's own account, so an
//! agent that only claims progress shows none.

use std::path::Path;
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LINES: usize = 25;
const MAX_CHECKS: usize = 8;
/// `git hash-object -t tree /dev/null`: the base for a repository with no commit before the goal.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub(crate) struct GoalEvidence {
    pub(crate) git_repo: bool,
    pub(crate) commits: String,
    pub(crate) diff_stat: String,
    pub(crate) uncommitted: String,
    pub(crate) checks: Vec<String>,
}

async fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The first `MAX_LINES` lines, with a count of the rest. For a diff stat the summary line comes
/// last, so it is kept even when the file list is cut.
fn bounded(text: &str, keep_last: bool) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_LINES {
        return text.to_string();
    }
    let mut out: Vec<String> = lines[..MAX_LINES].iter().map(|l| l.to_string()).collect();
    let hidden = lines.len() - MAX_LINES - usize::from(keep_last);
    out.push(format!("… {hidden} more"));
    if keep_last {
        out.push(lines[lines.len() - 1].to_string());
    }
    out.join("\n")
}

pub(crate) async fn collect(root: &Path, session_id: &str, started_unix: i64) -> GoalEvidence {
    let checks = passing_checks(root, session_id);
    if git(root, &["rev-parse", "--is-inside-work-tree"])
        .await
        .is_none()
    {
        return GoalEvidence {
            git_repo: false,
            commits: String::new(),
            diff_stat: String::new(),
            uncommitted: String::new(),
            checks,
        };
    }
    let since = format!("--since=@{started_unix}");
    let before = format!("--before=@{started_unix}");
    let commits = git(
        root,
        &["log", "--oneline", "--no-decorate", "-n", "30", &since],
    )
    .await
    .unwrap_or_default();
    let base = git(root, &["rev-list", "-1", &before, "HEAD"])
        .await
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| EMPTY_TREE.to_string());
    let diff_stat = git(root, &["diff", "--stat=100", &base])
        .await
        .unwrap_or_default();
    let uncommitted = git(root, &["status", "--short", "--untracked-files=normal"])
        .await
        .unwrap_or_default();
    GoalEvidence {
        git_repo: true,
        commits: bounded(&commits, false),
        diff_stat: bounded(&diff_stat, true),
        uncommitted: bounded(&uncommitted, false),
        checks,
    }
}

/// The most recent passing verification commands the evidence ledger recorded for this session.
fn passing_checks(root: &Path, session_id: &str) -> Vec<String> {
    let Some(ledger) = forge_tools::evidence::Ledger::new(root, session_id) else {
        return Vec::new();
    };
    let items = ledger.items();
    let start = items.len().saturating_sub(MAX_CHECKS);
    items[start..]
        .iter()
        .filter_map(|item| {
            let command = item.command.as_deref()?;
            Some(format!(
                "`{command}` exit {}",
                item.exit_code.map_or("?".to_string(), |c| c.to_string())
            ))
        })
        .collect()
}

impl GoalEvidence {
    pub(crate) fn render(&self) -> String {
        let or_none = |s: &str| {
            if s.trim().is_empty() {
                "(none)".to_string()
            } else {
                s.to_string()
            }
        };
        let mut out = String::new();
        if self.git_repo {
            out.push_str(&format!(
                "Commits since the goal started:\n{}\n\nDiff against the goal's starting point \
                 (committed and uncommitted):\n{}\n\nUncommitted files:\n{}\n",
                or_none(&self.commits),
                or_none(&self.diff_stat),
                or_none(&self.uncommitted)
            ));
        } else {
            out.push_str("Workspace is not a git repository: no commit or diff evidence.\n");
        }
        out.push_str(&format!(
            "\nPassing verification commands recorded this session:\n{}\n",
            if self.checks.is_empty() {
                "(none)".to_string()
            } else {
                self.checks
                    .iter()
                    .map(|c| format!("- {c}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str], date: Option<&str>) {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(dir)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args);
        if let Some(date) = date {
            cmd.env("GIT_AUTHOR_DATE", date)
                .env("GIT_COMMITTER_DATE", date);
        }
        assert!(cmd.output().unwrap().status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn commits_and_diff_since_the_goal_started_are_collected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        run(root, &["init", "-q"], None);
        run(root, &["config", "user.email", "t@t"], None);
        run(root, &["config", "user.name", "t"], None);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        run(root, &["add", "a.txt"], None);
        run(
            root,
            &["commit", "-qm", "before"],
            Some("@1000000000 +0000"),
        );
        std::fs::write(root.join("b.txt"), "two\n").unwrap();
        run(root, &["add", "b.txt"], None);
        run(root, &["commit", "-qm", "feat: add b"], None);
        std::fs::write(root.join("c.txt"), "three\n").unwrap();

        let started = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - 60;
        let evidence = collect(root, "s1", started).await;
        assert!(evidence.git_repo);
        assert!(
            evidence.commits.contains("feat: add b"),
            "{}",
            evidence.commits
        );
        assert!(!evidence.commits.contains("before"));
        assert!(
            evidence.diff_stat.contains("b.txt"),
            "{}",
            evidence.diff_stat
        );
        assert!(evidence.uncommitted.contains("c.txt"));
        assert!(evidence.render().contains("feat: add b"));
    }

    #[tokio::test]
    async fn a_plain_directory_says_there_is_no_git_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let evidence = collect(dir.path(), "s1", 0).await;
        assert!(!evidence.git_repo);
        assert!(evidence.render().contains("not a git repository"));
    }

    #[test]
    fn a_long_diff_stat_keeps_its_summary_line() {
        let text = (0..40)
            .map(|i| format!("f{i} | 1 +"))
            .chain(std::iter::once("40 files changed".to_string()))
            .collect::<Vec<_>>()
            .join("\n");
        let out = bounded(&text, true);
        assert!(out.ends_with("40 files changed"));
        assert!(out.contains("… 15 more"));
    }
}
