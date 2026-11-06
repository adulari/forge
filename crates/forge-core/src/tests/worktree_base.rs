use super::*;
use crate::worktree::WorktreeGuard;
use std::path::PathBuf;

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-qm", name, "--no-gpg-sign"]);
}

fn configured_clone(origin: &Path, at: &Path) {
    git(
        origin.parent().unwrap(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            at.to_str().unwrap(),
        ],
    );
    git(at, &["config", "user.email", "t@t"]);
    git(at, &["config", "user.name", "t"]);
}

/// A bare origin with `main`, a clone of it (`mine`, the checkout) and a second clone (`other`)
/// that can push ahead of it.
struct Fixture {
    _dir: tempfile::TempDir,
    mine: PathBuf,
    other: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let seed = root.join("seed");
    std::fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    git(&seed, &["config", "user.email", "t@t"]);
    git(&seed, &["config", "user.name", "t"]);
    commit(&seed, "one");
    let origin = root.join("origin.git");
    git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            origin.to_str().unwrap(),
        ],
    );
    let mine = root.join("mine");
    let other = root.join("other");
    configured_clone(&origin, &mine);
    configured_clone(&origin, &other);
    Fixture {
        _dir: dir,
        mine,
        other,
    }
}

fn push_ahead(f: &Fixture, n: usize) -> String {
    for i in 0..n {
        commit(&f.other, &format!("remote{i}"));
    }
    git(&f.other, &["push", "-q", "origin", "main"]);
    git(&f.other, &["rev-parse", "HEAD"])
}

#[test]
fn origin_ahead_becomes_the_base_and_the_note_counts_the_commits() {
    let f = fixture();
    let tip = push_ahead(&f, 3);
    let local_before = git(&f.mine, &["rev-parse", "HEAD"]);

    let base = fresh_base(&f.mine);
    assert_eq!(base.start_point, tip);
    assert_eq!(
        base.note,
        format!(
            "based on origin/main @{} (local main was 3 behind)",
            &tip[..7]
        )
    );

    let wt = f.mine.join(".forge/worktrees/fresh");
    let guard =
        WorktreeGuard::create_at_base(&f.mine, "fresh", wt.clone(), &base.start_point).unwrap();
    assert_eq!(git(&wt, &["rev-parse", "HEAD"]), tip);
    drop(guard);
    assert_eq!(git(&f.mine, &["rev-parse", "HEAD"]), local_before);
    assert_eq!(git(&f.mine, &["branch", "--show-current"]), "main");
    assert!(git(&f.mine, &["status", "--porcelain"]).is_empty());
}

#[test]
fn a_checkout_on_another_branch_keeps_head_and_says_why() {
    let f = fixture();
    push_ahead(&f, 2);
    git(&f.mine, &["checkout", "-q", "-b", "feature"]);
    let base = fresh_base(&f.mine);
    assert_eq!(base.start_point, "HEAD");
    assert!(base
        .note
        .starts_with("based on local feature (not the default branch main"));
    // Never consulted, so nothing was fetched.
    assert_eq!(
        git(&f.mine, &["rev-parse", "origin/main"]),
        git(&f.mine, &["rev-parse", "main"])
    );
}

#[test]
fn no_origin_remote_keeps_head() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().canonicalize().unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    commit(&repo, "one");
    let base = fresh_base(&repo);
    assert_eq!(base.start_point, "HEAD");
    assert_eq!(base.note, "based on local main (no origin remote)");
}

#[test]
fn a_failed_fetch_falls_back_to_head_with_the_reason_and_age() {
    let f = fixture();
    push_ahead(&f, 1);
    let origin = git(&f.mine, &["remote", "get-url", "origin"]);
    std::fs::remove_dir_all(&origin).unwrap();
    let base = fresh_base(&f.mine);
    assert_eq!(base.start_point, "HEAD");
    assert!(
        base.note.starts_with("based on local main; fetch failed: "),
        "{}",
        base.note
    );
    assert!(base.note.ends_with("since last fetch"), "{}", base.note);
}

#[test]
fn a_hung_fetch_is_cut_off_at_the_timeout() {
    let f = fixture();
    let hang = f._dir.path().join("hang.sh");
    std::fs::write(&hang, "#!/bin/sh\nsleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hang, std::fs::Permissions::from_mode(0o755)).unwrap();
        git(
            &f.mine,
            &["remote", "set-url", "origin", "ssh://nowhere.invalid/x.git"],
        );
        git(
            &f.mine,
            &["config", "core.sshCommand", hang.to_str().unwrap()],
        );
        let started = std::time::Instant::now();
        let base = fresh_base_with(&f.mine, Duration::from_millis(400));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(base.start_point, "HEAD");
        assert!(
            base.note.contains("fetch failed: timed out"),
            "{}",
            base.note
        );
    }
}

#[test]
fn a_diverged_local_default_is_not_replaced_by_origin() {
    let f = fixture();
    push_ahead(&f, 2);
    commit(&f.mine, "local-only");
    let base = fresh_base(&f.mine);
    assert_eq!(base.start_point, "HEAD");
    assert!(
        base.note
            .contains("diverged from origin/main (2 behind, 1 ahead)"),
        "{}",
        base.note
    );
}

#[test]
fn up_to_date_checkout_reports_so() {
    let f = fixture();
    let base = fresh_base(&f.mine);
    assert_eq!(base.start_point, "HEAD");
    assert!(
        base.note.ends_with("(up to date with origin/main)"),
        "{}",
        base.note
    );
}

#[test]
fn the_note_is_stored_in_the_git_dir_and_read_back() {
    let f = fixture();
    assert_eq!(stored_note(&f.mine), None);
    store_note(
        &f.mine,
        "based on origin/main @abc1234 (local main was 3 behind)",
    );
    assert_eq!(
        stored_note(&f.mine).as_deref(),
        Some("based on origin/main @abc1234 (local main was 3 behind)")
    );
    assert!(git(&f.mine, &["status", "--porcelain"]).is_empty());
}
