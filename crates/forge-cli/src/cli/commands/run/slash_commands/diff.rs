//! `/diff [path]`: the working tree's uncommitted changes against `HEAD`, staged or not, with
//! untracked files listed after the diff. Runs git on a blocking thread and posts the result.

use std::path::Path;
use std::process::Command;

use forge_tui::{App, Deferred};

pub(super) fn start(app: &mut App, workspace: Option<std::path::PathBuf>, path: String) {
    let Some(workspace) = workspace else {
        app.note("/diff: the session's workspace is not known yet — try again when the turn ends");
        return;
    };
    let inbox = app.deferred_inbox();
    tokio::task::spawn_blocking(move || inbox.post(collect(&workspace, path.trim())));
}

fn git(workspace: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn with_paths<'a>(head: &[&'a str], pathspec: &[&'a str]) -> Vec<&'a str> {
    head.iter().chain(pathspec).copied().collect()
}

fn collect(workspace: &Path, path: &str) -> Deferred {
    if git(workspace, &["rev-parse", "--is-inside-work-tree"]).is_none() {
        return Deferred::Notes(vec![format!(
            "/diff: {} is not inside a git repository",
            workspace.display()
        )]);
    }
    let pathspec: Vec<&str> = if path.is_empty() {
        Vec::new()
    } else {
        vec!["--", path]
    };
    // A repository with no commit yet has no HEAD to diff against: show what is staged instead.
    let has_head = git(workspace, &["rev-parse", "--verify", "-q", "HEAD"]).is_some();
    let base: &[&str] = if has_head {
        &["diff", "HEAD", "--no-color", "--no-ext-diff"]
    } else {
        &["diff", "--cached", "--no-color", "--no-ext-diff"]
    };
    let diff = git(workspace, &with_paths(base, &pathspec)).unwrap_or_default();
    let mut stat_args = base.to_vec();
    stat_args.push("--shortstat");
    let stat = git(workspace, &with_paths(&stat_args, &pathspec)).unwrap_or_default();
    let untracked = git(
        workspace,
        &with_paths(&["ls-files", "--others", "--exclude-standard"], &pathspec),
    )
    .unwrap_or_default();
    view(&diff, stat.trim(), &untracked)
}

/// What `/diff` shows for this git output.
fn view(diff: &str, shortstat: &str, untracked: &str) -> Deferred {
    let untracked: Vec<&str> = untracked.lines().filter(|l| !l.is_empty()).collect();
    if diff.trim().is_empty() && untracked.is_empty() {
        return Deferred::Notes(vec!["/diff: no uncommitted changes".to_string()]);
    }
    let mut summary = if shortstat.is_empty() {
        "no tracked changes".to_string()
    } else {
        shortstat.to_string()
    };
    let mut text = diff.trim_end().to_string();
    if !untracked.is_empty() {
        summary.push_str(&format!(" · {} untracked", untracked.len()));
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str("untracked files:\n");
        for file in &untracked {
            text.push_str(&format!("  {file}\n"));
        }
    }
    Deferred::View {
        title: "/diff  working tree vs HEAD".to_string(),
        summary,
        text,
        diff: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_tree_says_so_instead_of_opening_an_empty_view() {
        assert_eq!(
            view("", "", ""),
            Deferred::Notes(vec!["/diff: no uncommitted changes".to_string()])
        );
    }

    #[test]
    fn changes_and_untracked_files_open_one_coloured_view() {
        let Deferred::View {
            summary,
            text,
            diff,
            ..
        } = view(
            "diff --git a/x b/x\n+new\n",
            "1 file changed, 1 insertion(+)",
            "notes.md\n",
        )
        else {
            panic!("a view")
        };
        assert!(diff);
        assert_eq!(summary, "1 file changed, 1 insertion(+) · 1 untracked");
        assert!(text.starts_with("diff --git"));
        assert!(text.contains("untracked files:\n  notes.md"));
    }

    #[test]
    fn git_runs_against_a_real_repository() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        if !run(&["init", "-q"]).status.success() {
            return; // no git on this machine
        }
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        run(&["add", "a.txt"]);
        run(&[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "init",
        ]);
        assert_eq!(
            collect(dir.path(), ""),
            Deferred::Notes(vec!["/diff: no uncommitted changes".to_string()])
        );
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "new\n").unwrap();
        let Deferred::View { text, summary, .. } = collect(dir.path(), "") else {
            panic!("a view")
        };
        assert!(text.contains("-one") && text.contains("+two"), "{text}");
        assert!(text.contains("b.txt"));
        assert!(summary.contains("1 file changed"), "{summary}");
        // A pathspec narrows both the diff and the untracked list.
        let Deferred::View { text, .. } = collect(dir.path(), "b.txt") else {
            panic!("a view")
        };
        assert!(!text.contains("+two") && text.contains("b.txt"));
    }
}
