//! `forge worktree list|reclaim` — show what worktrees cost and reclaim the ones that are provably
//! safe to remove.
//!
//! The classification lives in `forge_core::worktree_reclaim`; this module is presentation plus the
//! two things only the CLI knows: which sessions are alive (the Store) and how much room is left on
//! the filesystem (`fs2`). Reclaim is a dry run unless `--yes` is passed, and even then it deletes
//! only what the classifier cleared.

use anyhow::{Context, Result};
use forge_core::worktree_reclaim::{self, human_age, human_bytes, Candidate, ReclaimReport};

use crate::cli::args::WorktreeOp;
use crate::open_store;

/// Below this fraction of free space, the report says so loudly instead of leaving the user to
/// notice when a linker dies with a bus error.
const LOW_FREE_FRACTION: f64 = 0.10;

pub(crate) fn worktree_cmd(op: WorktreeOp) -> Result<()> {
    let repo_root = repo_root()?;
    let report = survey(&repo_root)?;

    match op {
        WorktreeOp::List => {
            print_list(&repo_root, &report);
            Ok(())
        }
        WorktreeOp::Reclaim { yes } => reclaim(&repo_root, &report, yes),
    }
}

fn survey(repo_root: &std::path::Path) -> Result<ReclaimReport> {
    // A store that won't open is not a reason to refuse the survey — but it IS a reason not to
    // delete anything, since "no live sessions" would then be an assumption rather than a fact.
    let mut live: Vec<std::path::PathBuf> = open_store()
        .and_then(|s| s.live_session_paths().map_err(Into::into))
        .unwrap_or_default()
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect();
    // The worktree this command is being run from counts as in use, store or no store.
    live.extend(std::env::current_dir());
    worktree_reclaim::survey(repo_root, &live).map_err(|e| anyhow::anyhow!("{e}"))
}

fn repo_root() -> Result<std::path::PathBuf> {
    let cwd = std::env::current_dir()?;
    let out = std::process::Command::new("git")
        .args(["-C", &cwd.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .context("running git rev-parse")?;
    if !out.status.success() {
        anyhow::bail!("not inside a git repository — `forge worktree` needs one");
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim(),
    ))
}

fn print_list(repo_root: &std::path::Path, report: &ReclaimReport) {
    println!("⚒ worktrees for {}\n", repo_root.display());
    if report.candidates.is_empty() {
        println!("  no worktrees besides the main one");
        return;
    }

    println!("  {:>9}  {:>5}  {:<9}  PATH", "SIZE", "AGE", "STATE");
    for candidate in &report.candidates {
        let f = &candidate.facts;
        println!(
            "  {:>9}  {:>5}  {:<9}  {}{}",
            human_bytes(f.size_bytes),
            human_age(f.age_secs),
            if candidate.verdict.is_reclaim() {
                "reclaim"
            } else {
                "keep"
            },
            f.path.display(),
            f.branch
                .as_ref()
                .map(|b| format!(" ({b})"))
                .unwrap_or_default(),
        );
        println!(
            "  {:>9}  {:>5}  {:<9}  ↳ {}{}{}",
            "",
            "",
            "",
            candidate.verdict.reason(),
            if f.merged { ", merged" } else { "" },
            if f.orphaned {
                ", orphaned (owning session is gone)"
            } else {
                ""
            },
        );
    }
    println!();
    print_totals(repo_root, report);
}

fn print_totals(repo_root: &std::path::Path, report: &ReclaimReport) {
    let total = report.total_bytes();
    let reclaimable = report.reclaimable_bytes();
    println!(
        "  {} worktree(s), {} on disk, {} reclaimable",
        report.candidates.len(),
        human_bytes(total),
        human_bytes(reclaimable),
    );
    if let Some((free, capacity)) = free_space(repo_root) {
        let fraction = free as f64 / capacity.max(1) as f64;
        let line = format!(
            "  filesystem: {} free of {} ({:.0}%)",
            human_bytes(free),
            human_bytes(capacity),
            fraction * 100.0
        );
        if fraction < LOW_FREE_FRACTION {
            println!("{line}");
            println!(
                "  ⚠ LOW DISK — run `forge worktree reclaim --yes` to free {}",
                human_bytes(reclaimable)
            );
        } else {
            println!("{line}");
        }
    }
}

fn reclaim(repo_root: &std::path::Path, report: &ReclaimReport, apply: bool) -> Result<()> {
    let targets: Vec<&Candidate> = report.reclaimable().collect();
    println!("⚒ worktree reclaim for {}\n", repo_root.display());

    if targets.is_empty() {
        println!("  nothing is provably safe to reclaim");
    }
    for candidate in &targets {
        println!(
            "  {} {:>9}  {}  ({})",
            if apply { "removed" } else { "would remove" },
            human_bytes(candidate.facts.size_bytes),
            candidate.facts.path.display(),
            candidate.verdict.reason(),
        );
    }

    let mut freed = 0u64;
    let mut failures = 0usize;
    if apply {
        for candidate in &targets {
            match worktree_reclaim::remove(repo_root, &candidate.facts) {
                Ok(()) => freed = freed.saturating_add(candidate.facts.size_bytes),
                Err(e) => {
                    failures += 1;
                    println!("  ✗ {}: {e}", candidate.facts.path.display());
                }
            }
        }
    }

    let skipped: Vec<&Candidate> = report.skipped().collect();
    if !skipped.is_empty() {
        println!("\n  kept:");
        for candidate in skipped {
            println!(
                "  · {:>9}  {}  — {}",
                human_bytes(candidate.facts.size_bytes),
                candidate.facts.path.display(),
                candidate.verdict.reason(),
            );
        }
    }

    println!();
    if apply {
        println!("  freed {}", human_bytes(freed));
        if failures > 0 {
            println!("  {failures} removal(s) failed — see above");
        }
    } else {
        println!(
            "  dry run — nothing was deleted. {} would be freed; pass --yes to do it",
            human_bytes(report.reclaimable_bytes())
        );
    }
    Ok(())
}

/// Free and total bytes on the filesystem holding `path`.
fn free_space(path: &std::path::Path) -> Option<(u64, u64)> {
    let free = fs2::available_space(path).ok()?;
    let capacity = fs2::total_space(path).ok()?;
    Some((free, capacity))
}

/// One line for `forge doctor`: what worktrees cost right now, and whether the disk can take it.
/// `None` when the current directory isn't a git repository (doctor then says nothing about it).
pub(crate) fn doctor_summary() -> Option<crate::doctor::Check> {
    let repo_root = repo_root().ok()?;
    let report = survey(&repo_root).ok()?;
    let total = report.total_bytes();
    let reclaimable = report.reclaimable_bytes();
    let detail = format!(
        "{} worktree(s), {} on disk, {} reclaimable",
        report.candidates.len(),
        human_bytes(total),
        human_bytes(reclaimable),
    );

    let low_disk = free_space(&repo_root)
        .map(|(free, capacity)| (free as f64 / capacity.max(1) as f64) < LOW_FREE_FRACTION)
        .unwrap_or(false);

    // Worktrees are only worth flagging once they are actually expensive: a warning nobody can act
    // on trains people to ignore doctor.
    let status = if low_disk && reclaimable > 0 {
        crate::doctor::Status::Fail
    } else if reclaimable > 16 * 1024 * 1024 * 1024 {
        crate::doctor::Status::Warn
    } else {
        crate::doctor::Status::Info
    };
    let fix = if reclaimable > 0 {
        Some("`forge worktree reclaim` to preview, `--yes` to free it")
    } else {
        None
    };
    Some(crate::doctor::check(status, "worktree disk", detail, fix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::worktree_reclaim::{Verdict, WorktreeFacts};

    fn facts(size: u64) -> WorktreeFacts {
        WorktreeFacts {
            path: "/tmp/wt".into(),
            branch: Some("feat/x".into()),
            prunable: false,
            merged: true,
            dirty: false,
            unpushed: false,
            locked: false,
            live: false,
            forge_owned: true,
            orphaned: false,
            size_bytes: size,
            age_secs: Some(3_600),
        }
    }

    #[test]
    fn a_dry_run_reports_what_it_would_free_and_deletes_nothing() {
        let report = ReclaimReport {
            candidates: vec![Candidate {
                facts: facts(1024),
                verdict: Verdict::Reclaim("branch merged, working tree clean"),
            }],
        };
        assert_eq!(report.reclaimable_bytes(), 1024);
        // `apply = false` never reaches worktree_reclaim::remove — the path below would fail
        // loudly against /tmp/wt if it did.
        reclaim(std::path::Path::new("/nonexistent-repo"), &report, false).unwrap();
    }

    #[test]
    fn totals_separate_reclaimable_from_kept() {
        let report = ReclaimReport {
            candidates: vec![
                Candidate {
                    facts: facts(1000),
                    verdict: Verdict::Reclaim("branch merged, working tree clean"),
                },
                Candidate {
                    facts: facts(500),
                    verdict: Verdict::Skip("uncommitted changes".into()),
                },
            ],
        };
        assert_eq!(report.total_bytes(), 1500);
        assert_eq!(report.reclaimable_bytes(), 1000);
        assert_eq!(report.skipped().count(), 1);
    }
}
