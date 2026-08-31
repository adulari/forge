//! Reclaim safety, end to end against real git repositories.
//!
//! The rule under test is the one that matters: a merged, clean, unreferenced worktree is
//! reclaimed and NOTHING else is. Every refusal below (dirty tree, unpushed commits, live session)
//! represents work that would be unrecoverable if the classifier got it wrong.

use std::path::{Path, PathBuf};
use std::process::Command;

use forge_core::worktree_reclaim::{self, Verdict};

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repo with an `origin` remote (a bare clone) so merge and upstream checks are realistic.
fn init_repo(tag: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("forge-reclaim-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    let remote = base.join("remote.git");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&remote).unwrap();

    git(&remote, &["init", "--bare", "--initial-branch=main"]);
    git(&repo, &["init", "--initial-branch=main"]);
    git(&repo, &["config", "user.email", "test@forge.local"]);
    git(&repo, &["config", "user.name", "Forge Test"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repo.join("README"), "reclaim test\n").unwrap();
    // Build output is ignored, as in any real repo: a worktree holding only artifacts is clean.
    std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
    git(&repo, &["add", "README", ".gitignore"]);
    git(&repo, &["commit", "-m", "init", "--no-gpg-sign"]);
    git(
        &repo,
        &["remote", "add", "origin", &remote.to_string_lossy()],
    );
    git(&repo, &["push", "-u", "origin", "main"]);
    repo
}

/// Add a worktree on a new branch. `merged` keeps it at main's tip (an ancestor of origin/main).
fn add_worktree(repo: &Path, name: &str) -> PathBuf {
    let path = repo.join(".forge").join("worktrees").join(name);
    git(
        repo,
        &[
            "worktree",
            "add",
            &path.to_string_lossy(),
            "-b",
            &format!("forge/subagent/{name}"),
            "HEAD",
        ],
    );
    git(&path, &["config", "user.email", "test@forge.local"]);
    git(&path, &["config", "user.name", "Forge Test"]);
    path
}

fn find<'a>(
    report: &'a worktree_reclaim::ReclaimReport,
    path: &Path,
) -> &'a worktree_reclaim::Candidate {
    report
        .candidates
        .iter()
        .find(|c| c.facts.path == path)
        .unwrap_or_else(|| panic!("{} missing from the survey", path.display()))
}

#[test]
fn a_merged_clean_worktree_is_reclaimed() {
    if !git_available() {
        return;
    }
    let repo = init_repo("merged");
    let wt = add_worktree(&repo, "merged");

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    let candidate = find(&report, &wt);
    assert!(
        candidate.verdict.is_reclaim(),
        "merged + clean must be reclaimable, got: {:?}",
        candidate.verdict
    );

    worktree_reclaim::remove(&repo, &candidate.facts).unwrap();
    assert!(!wt.exists(), "the worktree directory must be gone");
    let listed = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(
        !listed.contains(&wt.to_string_lossy().to_string()),
        "the registration must be gone too:\n{listed}"
    );

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn a_dirty_worktree_is_refused() {
    if !git_available() {
        return;
    }
    let repo = init_repo("dirty");
    let wt = add_worktree(&repo, "dirty");
    // Four dirty files: real in-progress work, exactly the case that must never be deleted.
    for n in 0..4 {
        std::fs::write(wt.join(format!("wip{n}.txt")), "in progress\n").unwrap();
    }

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    let candidate = find(&report, &wt);
    assert!(candidate.facts.dirty);
    assert_eq!(
        candidate.verdict,
        Verdict::Skip("uncommitted changes".into())
    );

    // Even asked directly, removal must refuse.
    assert!(worktree_reclaim::remove(&repo, &candidate.facts).is_err());
    assert!(wt.join("wip0.txt").exists(), "dirty work must survive");

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn a_worktree_with_unpushed_commits_is_refused() {
    if !git_available() {
        return;
    }
    let repo = init_repo("unpushed");
    let wt = add_worktree(&repo, "unpushed");
    // Publish the branch, then commit on top: ahead of its upstream, clean working tree.
    git(&wt, &["push", "-u", "origin", "HEAD"]);
    std::fs::write(wt.join("work.txt"), "committed but unpushed\n").unwrap();
    git(&wt, &["add", "work.txt"]);
    git(&wt, &["commit", "-m", "unpushed work", "--no-gpg-sign"]);

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    let candidate = find(&report, &wt);
    assert!(!candidate.facts.dirty, "working tree is clean");
    assert!(candidate.facts.unpushed);
    assert_eq!(
        candidate.verdict,
        Verdict::Skip("unpushed commits".into()),
        "an unpushed commit outranks every reason to delete"
    );

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn a_live_session_pins_an_otherwise_reclaimable_worktree() {
    if !git_available() {
        return;
    }
    let repo = init_repo("live");
    let wt = add_worktree(&repo, "live");

    let report = worktree_reclaim::survey(&repo, &[wt.join("src")]).unwrap();
    let candidate = find(&report, &wt);
    assert_eq!(
        candidate.verdict,
        Verdict::Skip("a live session is using it".into())
    );
    assert!(worktree_reclaim::remove(&repo, &candidate.facts).is_err());

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn a_prunable_registration_is_pruned() {
    if !git_available() {
        return;
    }
    let repo = init_repo("prunable");
    let wt = add_worktree(&repo, "prunable");
    // Delete the directory by hand — the registration lingers, which is 10 of the 76 real ones.
    std::fs::remove_dir_all(&wt).unwrap();

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    let candidate = find(&report, &wt);
    assert!(candidate.facts.prunable);
    assert_eq!(
        candidate.verdict,
        Verdict::Reclaim("stale registration (directory is gone)")
    );

    worktree_reclaim::remove(&repo, &candidate.facts).unwrap();
    let listed = git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(
        !listed.contains(&wt.to_string_lossy().to_string()),
        "the stale registration must be pruned:\n{listed}"
    );

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn survey_reports_size_and_sorts_the_worst_offender_first() {
    if !git_available() {
        return;
    }
    let repo = init_repo("sizes");
    let small = add_worktree(&repo, "small");
    let big = add_worktree(&repo, "big");
    // Build artifacts are where the gigabytes are; they must be counted.
    std::fs::create_dir_all(big.join("target").join("debug")).unwrap();
    std::fs::write(
        big.join("target").join("debug").join("blob"),
        vec![7u8; 200_000],
    )
    .unwrap();
    std::fs::create_dir_all(small.join("target")).unwrap();
    std::fs::write(small.join("target").join("blob"), vec![7u8; 1_000]).unwrap();

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    assert!(find(&report, &big).facts.size_bytes > 200_000);
    assert!(find(&report, &big).facts.artifact_bytes >= 200_000);
    assert!(find(&report, &small).facts.artifact_bytes >= 1_000);
    assert!(report.artifact_bytes() >= 201_000);
    assert_eq!(
        report.candidates.first().map(|c| c.facts.path.clone()),
        Some(big.clone()),
        "the biggest reclaimable worktree must sort first"
    );
    assert!(report.reclaimable_bytes() > 200_000);
    assert!(report.total_bytes() >= report.reclaimable_bytes());

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn artifacts_in_a_kept_worktree_are_prunable_without_touching_source() {
    if !git_available() {
        return;
    }
    let repo = init_repo("kept-artifacts");
    let wt = add_worktree(&repo, "kept-artifacts");
    std::fs::write(wt.join("wip.txt"), "keep me\n").unwrap();
    std::fs::create_dir_all(wt.join("target/debug")).unwrap();
    std::fs::write(wt.join("target/debug/blob"), vec![1u8; 4096]).unwrap();

    let report = worktree_reclaim::survey_with_artifact_age(&repo, &[], 0).unwrap();
    let candidate = find(&report, &wt);
    assert_eq!(
        candidate.verdict,
        Verdict::Skip("uncommitted changes".into())
    );
    assert!(candidate.artifact_verdict.is_reclaim());

    assert_eq!(
        worktree_reclaim::prune_artifacts(&wt, &[], 0).unwrap(),
        4096
    );
    assert!(!wt.join("target").exists());
    assert_eq!(
        std::fs::read_to_string(wt.join("wip.txt")).unwrap(),
        "keep me\n"
    );

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn live_session_refuses_artifact_pruning() {
    if !git_available() {
        return;
    }
    let repo = init_repo("live-artifacts");
    let wt = add_worktree(&repo, "live-artifacts");
    std::fs::create_dir_all(wt.join("target")).unwrap();
    std::fs::write(wt.join("target/blob"), vec![1u8; 4096]).unwrap();

    let report =
        worktree_reclaim::survey_with_artifact_age(&repo, std::slice::from_ref(&wt), 0).unwrap();
    let candidate = find(&report, &wt);
    assert_eq!(
        candidate.artifact_verdict,
        Verdict::Skip("a live session is using this worktree".into())
    );
    assert!(worktree_reclaim::prune_artifacts(&wt, std::slice::from_ref(&wt), 0).is_err());
    assert!(wt.join("target/blob").exists());

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn artifact_dry_run_observation_deletes_nothing() {
    if !git_available() {
        return;
    }
    let repo = init_repo("artifact-dry-run");
    let wt = add_worktree(&repo, "artifact-dry-run");
    std::fs::create_dir_all(wt.join("target")).unwrap();
    std::fs::write(wt.join("target/blob"), vec![1u8; 4096]).unwrap();

    let report = worktree_reclaim::survey_with_artifact_age(&repo, &[], 0).unwrap();
    assert!(find(&report, &wt).artifact_verdict.is_reclaim());
    assert!(wt.join("target/blob").exists());

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn an_ownership_record_survives_a_process_that_never_ran_drop() {
    if !git_available() {
        return;
    }
    let repo = init_repo("orphan");
    let wt = add_worktree(&repo, "orphan");
    // pid 0 is never a live process: the signature of a session killed before Drop.
    worktree_reclaim::record_ownership(&repo, "orphan", &wt, "forge/subagent/orphan");
    let owners = worktree_reclaim::read_owners(&repo);
    assert_eq!(
        owners.len(),
        1,
        "the record must be readable after the fact"
    );

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    let candidate = find(&report, &wt);
    assert!(
        candidate.facts.forge_owned,
        "the record identifies the worktree as Forge's"
    );

    worktree_reclaim::clear_ownership(&repo, "orphan");
    assert!(worktree_reclaim::read_owners(&repo).is_empty());

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn an_unmerged_branch_is_refused_with_the_reason() {
    if !git_available() {
        return;
    }
    let repo = init_repo("unmerged");
    let wt = add_worktree(&repo, "unmerged");
    std::fs::write(wt.join("only-here.txt"), "not on main\n").unwrap();
    git(&wt, &["add", "only-here.txt"]);
    git(&wt, &["commit", "-m", "diverge", "--no-gpg-sign"]);

    let report = worktree_reclaim::survey(&repo, &[]).unwrap();
    let candidate = find(&report, &wt);
    assert!(!candidate.facts.merged);
    match &candidate.verdict {
        Verdict::Skip(reason) => assert!(reason.contains("not merged"), "reason: {reason}"),
        other => panic!("an unmerged branch must be skipped, got {other:?}"),
    }

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

/// Regression, caught the first time this ran against a real repository: surveying FROM a linked
/// worktree listed the main checkout — on `main`, merged, clean — as reclaimable. Deleting it would
/// have taken the repository with it.
#[test]
fn the_main_checkout_is_never_a_candidate() {
    if !git_available() {
        return;
    }
    let repo = init_repo("main-checkout");
    let wt = add_worktree(&repo, "linked");

    for root in [repo.as_path(), wt.as_path()] {
        let report = worktree_reclaim::survey(root, &[]).unwrap();
        assert!(
            !report.candidates.iter().any(|c| c.facts.path == repo),
            "the main checkout must not appear when surveying from {}",
            root.display()
        );
    }

    std::fs::remove_dir_all(repo.parent().unwrap()).ok();
}

#[test]
fn human_bytes_reads_like_a_disk_report() {
    assert_eq!(worktree_reclaim::human_bytes(512), "512 B");
    assert_eq!(worktree_reclaim::human_bytes(1536), "1.5 KB");
    assert_eq!(
        worktree_reclaim::human_bytes(475 * 1024 * 1024 * 1024),
        "475.0 GB"
    );
}
