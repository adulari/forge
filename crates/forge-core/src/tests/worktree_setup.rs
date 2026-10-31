use super::*;
use forge_config::worktree::{WorktreeConfig, WorktreeRoot};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo(dir: &Path) -> PathBuf {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["config", "user.name", "t"]);
    std::fs::write(root.join(".gitignore"), ".env*\n.vscode/\nnode_modules/\n").unwrap();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "init"]);
    root.canonicalize().unwrap()
}

#[test]
fn copies_only_ignored_files_that_match_and_runs_setup() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    std::fs::write(root.join(".env"), "KEY=1\n").unwrap();
    std::fs::write(root.join(".env.local"), "KEY=2\n").unwrap();
    std::fs::create_dir_all(root.join(".vscode")).unwrap();
    std::fs::write(root.join(".vscode/settings.json"), "{}").unwrap();
    std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
    std::fs::write(root.join("node_modules/x/i.js"), "x").unwrap();
    let wt = dir.path().join("wt");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "t", wt.to_str().unwrap()],
    );
    let cfg = WorktreeConfig {
        copy_ignored: vec![".env*".into(), ".vscode".into(), "a.txt".into()],
        setup: Some("echo ran-$FORGE_WORKTREE_BRANCH > .setup-ran".into()),
        ..Default::default()
    };
    let out = provision(&root, &wt, "t", &cfg);
    assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "KEY=1\n");
    assert!(wt.join(".env.local").is_file());
    assert!(wt.join(".vscode/settings.json").is_file());
    assert!(!wt.join("node_modules").exists(), "not in copy_ignored");
    assert!(out.setup.ran && out.setup.ok, "{:?}", out.setup);
    assert_eq!(
        std::fs::read_to_string(wt.join(".setup-ran"))
            .unwrap()
            .trim(),
        "ran-t"
    );
    assert!(out.copied.contains(&".env".to_string()));
    assert!(out.summary().contains("setup ran"));
}

#[test]
fn a_failing_setup_is_reported_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let cfg = WorktreeConfig {
        setup: Some("echo boom >&2; exit 3".into()),
        ..Default::default()
    };
    let out = provision(&root, &root, "main", &cfg);
    assert!(out.setup.ran && !out.setup.ok);
    assert!(out.setup.output.contains("boom"));
    assert!(out.summary().contains("setup FAILED"));
}

#[test]
fn patterns_cannot_escape_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let wt = dir.path().join("wt");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "t", wt.to_str().unwrap()],
    );
    let cfg = WorktreeConfig {
        copy_ignored: vec!["../*".into(), "/etc/passwd".into()],
        ..Default::default()
    };
    let out = provision(&root, &wt, "t", &cfg);
    assert!(out.copied.is_empty());
    assert_eq!(out.skipped.len(), 2);
}

#[test]
fn cleanup_runs_in_the_worktree_and_is_a_noop_unset() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let marker = dir.path().join("cleaned");
    let cfg = WorktreeConfig {
        cleanup: Some(format!("pwd > {}", marker.display())),
        ..Default::default()
    };
    assert!(teardown(&root, &root, "main", &cfg).ok);
    assert!(std::fs::read_to_string(marker)
        .unwrap()
        .trim()
        .ends_with("repo"));
    assert!(!teardown(&root, &root, "main", &WorktreeConfig::default()).ran);
}

#[test]
fn project_root_is_inside_the_repo_and_global_root_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let project = worktree_path(&root, &WorktreeConfig::default(), "x");
    assert_eq!(project, root.join(".forge/worktrees/x"));
    let global = worktree_path(
        &root,
        &WorktreeConfig {
            root: WorktreeRoot::Global,
            ..Default::default()
        },
        "x",
    );
    assert!(!global.starts_with(&root));
    assert!(global.ends_with("x"));
    assert!(global
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("repo-"));
}

#[test]
fn friendly_names_are_readable_slugs_and_vary() {
    let names: std::collections::HashSet<String> = (0..20).map(|_| friendly_name()).collect();
    assert!(names.len() > 10);
    for name in names {
        assert_eq!(crate::worktree_ops::slug(&name), name);
        assert_eq!(name.split('-').count(), 3, "{name}");
    }
}

#[cfg(unix)]
#[test]
fn a_timeout_kills_grandchildren_and_returns() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let started = Instant::now();
    let out = run_hook_within(
        "sleep 300 & sleep 300; echo never",
        &root,
        &root,
        "main",
        Duration::from_millis(300),
        None,
    );
    assert!(started.elapsed() < Duration::from_secs(20), "hung");
    assert!(out.ran && !out.ok);
    assert!(out.output.contains("timed out"), "{}", out.output);
}

#[cfg(unix)]
#[test]
fn a_backgrounded_process_does_not_hang_the_hook() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let started = Instant::now();
    let out = run_hook("echo started; sleep 300 &", &root, &root, "main");
    assert!(started.elapsed() < Duration::from_secs(20), "hung");
    assert!(out.ok, "{out:?}");
    assert!(out.output.contains("started"));
}

#[cfg(unix)]
#[test]
fn copy_ignored_never_writes_through_a_tracked_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("cfg")).unwrap();
    git(&root, &["add", "cfg"]);
    git(&root, &["commit", "-qm", "link"]);
    std::fs::write(root.join(".env"), "SECRET=1\n").unwrap();
    let wt = dir.path().join("wt");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "t", wt.to_str().unwrap()],
    );
    std::os::unix::fs::symlink(outside.join("stolen"), wt.join(".env")).unwrap();
    let cfg = WorktreeConfig {
        copy_ignored: vec![".env".into()],
        ..Default::default()
    };
    let out = provision(&root, &wt, "t", &cfg);
    assert!(out.copied.is_empty(), "{out:?}");
    assert!(!outside.join("stolen").exists());
}

#[test]
fn copy_ignored_finds_files_inside_wholly_ignored_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    std::fs::create_dir_all(root.join(".vscode")).unwrap();
    std::fs::write(root.join(".vscode/settings.json"), "{}").unwrap();
    std::fs::write(root.join(".vscode/other.json"), "{}").unwrap();
    let wt = dir.path().join("wt");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "t", wt.to_str().unwrap()],
    );
    let cfg = WorktreeConfig {
        copy_ignored: vec![".vscode/settings.json".into()],
        ..Default::default()
    };
    let out = provision(&root, &wt, "t", &cfg);
    assert!(wt.join(".vscode/settings.json").is_file(), "{out:?}");
    assert!(!wt.join(".vscode/other.json").exists());
}

#[test]
fn an_oversized_directory_is_skipped_whole_not_half_copied() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    std::fs::create_dir_all(root.join("node_modules/a")).unwrap();
    std::fs::write(root.join("node_modules/a/small"), "x").unwrap();
    let big = std::fs::File::create(root.join("node_modules/big")).unwrap();
    big.set_len(COPY_BUDGET_BYTES + 1).unwrap();
    let wt = dir.path().join("wt");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "t", wt.to_str().unwrap()],
    );
    let cfg = WorktreeConfig {
        copy_ignored: vec!["node_modules".into()],
        ..Default::default()
    };
    let out = provision(&root, &wt, "t", &cfg);
    assert!(!wt.join("node_modules").exists(), "{out:?}");
    assert_eq!(out.skipped.len(), 1);
}

#[test]
fn friendly_names_reach_the_whole_word_lists() {
    let names: std::collections::HashSet<String> = (0..4000).map(|_| friendly_name()).collect();
    let adjectives: std::collections::HashSet<&str> =
        names.iter().filter_map(|n| n.split('-').next()).collect();
    assert!(adjectives.len() > 20, "{}", adjectives.len());
}

#[test]
fn copy_ignored_does_not_reach_into_sibling_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    std::fs::write(root.join(".env"), "KEY=1\n").unwrap();
    git(&root, &["config", "core.excludesFile", "/dev/null"]);
    std::fs::write(root.join(".git/info/exclude"), ".forge/\n.env*\n").unwrap();
    let sibling = root.join(".forge/worktrees/old");
    git(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "old",
            sibling.to_str().unwrap(),
        ],
    );
    std::fs::write(sibling.join(".env"), "OLD=1\n").unwrap();
    let wt = root.join(".forge/worktrees/new");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "new", wt.to_str().unwrap()],
    );
    let cfg = WorktreeConfig {
        copy_ignored: vec![".env*".into()],
        ..Default::default()
    };
    let out = provision(&root, &wt, "new", &cfg);
    assert_eq!(out.copied, vec![".env".to_string()], "{out:?}");
    assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "KEY=1\n");
}

#[test]
fn a_cancelled_provision_stops_copying() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    std::fs::write(root.join(".env"), "KEY=1\n").unwrap();
    let wt = dir.path().join("wt");
    git(
        &root,
        &["worktree", "add", "-q", "-b", "t", wt.to_str().unwrap()],
    );
    let control = SetupControl::default();
    control.cancel();
    let cfg = WorktreeConfig {
        copy_ignored: vec![".env".into()],
        setup: Some("touch ran".into()),
        ..Default::default()
    };
    let out = provision_with(&root, &wt, "t", &cfg, Some(&control));
    assert!(out.copied.is_empty() && !out.setup.ran, "{out:?}");
    assert!(!wt.join(".env").exists());
}
