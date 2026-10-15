//! Runs the scenario against a [`Subject`]: the scripted turns and compactions, then the probes
//! (before and after a resume), each probe independent of the others.
//!
//! The subject is a trait so the plumbing is testable with a scripted fake; the live one wraps a
//! real `Session` (`live.rs`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::scenario::{
    all_probes, zone_of, Check, Probe, Turn, Zone, ASK_PREFIX, EXTRA_WORK, SCENARIO,
};
use super::score::{cap, CompactionRecord, Phase, ProbeResult, UsageTotals};

pub(super) struct TurnOutcome {
    pub text: String,
    pub tool_calls: u64,
    pub error: Option<String>,
}

pub(super) trait Subject {
    async fn turn(&mut self, prompt: &str) -> TurnOutcome;
    /// Fold the transcript exactly as `/compact` does; returns (messages before, after).
    async fn compact(&mut self) -> Result<(usize, usize)>;
    /// Drop the last turn from the session, like `/undo`.
    async fn undo(&mut self) -> Result<()>;
    /// Replace the live session with one rebuilt from the store.
    async fn resume(&mut self) -> Result<()>;
    fn workdir(&self) -> &Path;
    fn usage(&self) -> UsageTotals;
    fn session_id(&self) -> Option<String>;
}

pub(super) enum Step {
    Say(String),
    Compact(&'static str),
}

fn zone_prompts(zone: Zone) -> Vec<&'static str> {
    SCENARIO
        .iter()
        .filter(|t| t.zone == zone)
        .map(|t: &Turn| t.prompt)
        .collect()
}

/// `base` scaled to `round(len * scale)` turns: truncated below 1x, padded above it by cycling
/// [`EXTRA_WORK`] with a pass number.
fn scaled(base: Vec<&'static str>, scale: f64) -> Vec<String> {
    let want = (base.len() as f64 * scale.max(0.0)).round() as usize;
    let mut out: Vec<String> = base.iter().take(want).map(|s| s.to_string()).collect();
    let mut i = 0;
    while out.len() < want {
        out.push(format!(
            "{} (pass {})",
            EXTRA_WORK[i % EXTRA_WORK.len()],
            i / EXTRA_WORK.len() + 2
        ));
        i += 1;
    }
    out
}

pub(super) fn plan(scale: f64) -> Vec<Step> {
    let say = |prompts: Vec<String>| prompts.into_iter().map(Step::Say).collect::<Vec<_>>();
    let fixed = |zone| say(zone_prompts(zone).into_iter().map(String::from).collect());
    let mut steps = fixed(Zone::Plant);
    steps.extend(say(scaled(zone_prompts(Zone::Work), scale)));
    steps.push(Step::Compact("mid-scenario"));
    steps.extend(fixed(Zone::Late));
    steps.extend(say(scaled(zone_prompts(Zone::Wrap), scale)));
    steps.push(Step::Compact("near-end"));
    steps
}

pub(super) struct Outcome {
    pub scenario_turns: usize,
    pub failed_turns: usize,
    pub compactions: Vec<CompactionRecord>,
    pub probes: Vec<ProbeResult>,
    pub resumed: bool,
}

pub(super) async fn run<S: Subject>(
    subject: &mut S,
    scale: f64,
    skip_resume: bool,
) -> Result<Outcome> {
    let steps = plan(scale);
    let total = steps.iter().filter(|s| matches!(s, Step::Say(_))).count();
    let mut out = Outcome {
        scenario_turns: total,
        failed_turns: 0,
        compactions: Vec::new(),
        probes: Vec::new(),
        resumed: false,
    };
    let mut n = 0;
    for step in steps {
        match step {
            Step::Say(prompt) => {
                n += 1;
                eprintln!("[{n}/{total}] {}", cap(&prompt, 70));
                let turn = subject.turn(&prompt).await;
                if let Some(e) = turn.error {
                    out.failed_turns += 1;
                    eprintln!("  turn failed: {e}");
                }
            }
            Step::Compact(label) => {
                eprintln!("compacting ({label})");
                let record = match subject.compact().await {
                    Ok((before, after)) => CompactionRecord {
                        label,
                        messages_before: before,
                        messages_after: after,
                        error: None,
                    },
                    Err(e) => CompactionRecord {
                        label,
                        messages_before: 0,
                        messages_after: 0,
                        error: Some(format!("{e:#}")),
                    },
                };
                out.compactions.push(record);
            }
        }
    }
    probe_phase(subject, Phase::AfterCompaction, &mut out.probes).await?;
    if !skip_resume {
        eprintln!("resuming from the store");
        subject.resume().await.context("resuming the session")?;
        out.resumed = true;
        probe_phase(subject, Phase::AfterResume, &mut out.probes).await?;
    }
    Ok(out)
}

async fn probe_phase<S: Subject>(
    subject: &mut S,
    phase: Phase,
    results: &mut Vec<ProbeResult>,
) -> Result<()> {
    eprintln!("probing ({})", phase.label());
    for probe in all_probes() {
        let result = run_probe(subject, probe, phase).await?;
        eprintln!(
            "  {} {} ({})",
            if result.passed { "pass" } else { "FAIL" },
            probe.id,
            cap(&result.observed, 90)
        );
        results.push(result);
    }
    Ok(())
}

/// One probe in isolation: the workspace is snapshotted first and the turn is undone and the
/// workspace restored afterwards, so neither the question nor its answer (nor a stray edit) is
/// visible to the next probe, or to the resume that follows the first phase.
async fn run_probe<S: Subject>(
    subject: &mut S,
    probe: &Probe,
    phase: Phase,
) -> Result<ProbeResult> {
    let workdir = subject.workdir().to_path_buf();
    let snapshot = Snapshot::take(&workdir)?;
    let head_before = head(&workdir).ok();
    let prompt = match probe.check {
        Check::Answer(_) => format!("{ASK_PREFIX}{}", probe.prompt),
        _ => probe.prompt.to_string(),
    };
    let turn = subject.turn(&prompt).await;
    let (passed, observed) = evaluate(&probe.check, &turn.text, &workdir, head_before.as_deref());
    let mut error = turn.error;
    if let Err(e) = subject.undo().await {
        error.get_or_insert_with(|| format!("undo failed: {e:#}"));
    }
    snapshot.restore()?;
    Ok(ProbeResult {
        id: probe.id.to_string(),
        category: probe.category,
        zone: zone_of(probe.id),
        phase,
        passed,
        observed,
        tool_calls: turn.tool_calls,
        error,
    })
}

pub(super) fn evaluate(
    check: &Check,
    reply: &str,
    workdir: &Path,
    head_before: Option<&str>,
) -> (bool, String) {
    match check {
        Check::Answer(matcher) => (matcher.accepts(reply), cap(reply, 200)),
        Check::File { path, matcher } => match std::fs::read_to_string(workdir.join(path)) {
            Ok(body) => {
                let ok = matcher.accepts(&body);
                let verdict = if ok {
                    "file ok"
                } else {
                    "file violates the rule"
                };
                (ok, format!("{verdict} | reply: {}", cap(reply, 120)))
            }
            Err(e) => (false, format!("cannot read {path}: {e}")),
        },
        Check::CommitSubject(matcher) => {
            let now = head(workdir).ok();
            if now.is_none() || now.as_deref() == head_before {
                return (false, format!("no new commit | reply: {}", cap(reply, 120)));
            }
            let subject = crate::bench::run_git(workdir, &["log", "-1", "--format=%s"])
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            (matcher.accepts(&subject), format!("commit: {subject}"))
        }
    }
}

fn head(workdir: &Path) -> Result<String> {
    crate::bench::run_git(workdir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

/// A copy of the workspace (git state included) taken before a probe.
struct Snapshot {
    dir: PathBuf,
    copy: tempfile::TempDir,
}

impl Snapshot {
    fn take(dir: &Path) -> Result<Self> {
        let copy = tempfile::Builder::new()
            .prefix("forge-recall-snap-")
            .tempdir()
            .context("creating snapshot dir")?;
        copy_tree(dir, copy.path())?;
        Ok(Self {
            dir: dir.to_path_buf(),
            copy,
        })
    }

    fn restore(self) -> Result<()> {
        for entry in std::fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        copy_tree(self.copy.path(), &self.dir)
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg(format!("{}/.", from.display()))
        .arg(to)
        .status()
        .context("running cp -a")?;
    anyhow::ensure!(
        status.success(),
        "cp -a {} -> {} failed",
        from.display(),
        to.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::fixture;
    use super::super::scenario::{Category, BEHAVIOR_PROBES};
    use super::super::score::{score_phase, Tally};
    use super::*;

    #[test]
    fn plan_orders_zones_around_the_two_compactions() {
        let steps = plan(1.0);
        let compacts: Vec<usize> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Step::Compact(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(compacts.len(), 2);
        assert_eq!(
            compacts[1],
            steps.len() - 1,
            "the second compaction is last"
        );
        let says = steps.iter().filter(|s| matches!(s, Step::Say(_))).count();
        assert_eq!(says, SCENARIO.len());
        assert!((15..=25).contains(&says), "scenario size: {says}");
        // Early facts are all planted before the first compaction; late ones after it.
        let position = |needle: &str| {
            steps
                .iter()
                .position(|s| matches!(s, Step::Say(p) if p.contains(needle)))
                .expect(needle)
        };
        assert!(position("hand-rolled") < compacts[0]);
        assert!(position("# Errors") > compacts[0]);
    }

    #[test]
    fn turns_scale_truncates_and_pads_filler_but_never_the_planted_turns() {
        let count = |scale: f64| {
            plan(scale)
                .iter()
                .filter(|s| matches!(s, Step::Say(_)))
                .count()
        };
        let planted = zone_prompts(Zone::Plant).len() + zone_prompts(Zone::Late).len();
        let filler = zone_prompts(Zone::Work).len() + zone_prompts(Zone::Wrap).len();
        assert_eq!(count(0.0), planted);
        assert_eq!(count(1.0), planted + filler);
        assert_eq!(count(2.0), planted + 2 * filler);
        let padded = plan(3.0);
        assert!(padded
            .iter()
            .any(|s| matches!(s, Step::Say(p) if p.ends_with("(pass 2)"))));
    }

    #[test]
    fn probe_ids_and_questions_are_unique() {
        let mut ids = HashSet::new();
        let mut prompts = HashSet::new();
        for p in all_probes() {
            assert!(ids.insert(p.id), "duplicate probe id {}", p.id);
            assert!(prompts.insert(p.prompt), "duplicate probe prompt {}", p.id);
        }
        assert!(BEHAVIOR_PROBES
            .iter()
            .all(|p| !matches!(p.check, Check::Answer(_))));
    }

    #[test]
    fn evaluate_checks_files_and_commits_in_a_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        fixture::create(tmp.path()).unwrap();
        let wd = tmp.path();
        let before = head(wd).unwrap();
        let unwrap_probe = BEHAVIOR_PROBES
            .iter()
            .find(|p| p.id == "obey_unwrap_rule")
            .unwrap();
        let commit_probe = BEHAVIOR_PROBES
            .iter()
            .find(|p| p.id == "obey_commit_prefix")
            .unwrap();

        let (ok, _) = evaluate(&unwrap_probe.check, "done", wd, Some(&before));
        assert!(!ok, "the function does not exist yet");
        std::fs::write(wd.join("src/parser.rs"), unwrap_probe.bad).unwrap();
        assert!(!evaluate(&unwrap_probe.check, "done", wd, Some(&before)).0);
        std::fs::write(wd.join("src/parser.rs"), unwrap_probe.good).unwrap();
        assert!(evaluate(&unwrap_probe.check, "done", wd, Some(&before)).0);

        assert!(
            !evaluate(&commit_probe.check, "done", wd, Some(&before)).0,
            "no new commit"
        );
        crate::bench::run_git(wd, &["commit", "-qam", "fix: no prefix"]).unwrap();
        assert!(
            !evaluate(&commit_probe.check, "done", wd, Some(&before)).0,
            "wrong prefix"
        );
        std::fs::write(wd.join("src/limits.rs"), "// changed\n").unwrap();
        crate::bench::run_git(wd, &["commit", "-qam", "JIRA-123: right prefix"]).unwrap();
        assert!(evaluate(&commit_probe.check, "done", wd, Some(&before)).0);
    }

    #[test]
    fn a_snapshot_restores_edits_new_files_and_commits() {
        let tmp = tempfile::tempdir().unwrap();
        fixture::create(tmp.path()).unwrap();
        let wd = tmp.path();
        let head_before = head(wd).unwrap();
        let snap = Snapshot::take(wd).unwrap();
        std::fs::write(wd.join("src/limits.rs"), "changed").unwrap();
        std::fs::write(wd.join("new.txt"), "x").unwrap();
        crate::bench::run_git(wd, &["commit", "-qam", "extra"]).unwrap();
        snap.restore().unwrap();
        assert_eq!(head(wd).unwrap(), head_before);
        assert!(!wd.join("new.txt").exists());
        assert!(std::fs::read_to_string(wd.join("src/limits.rs"))
            .unwrap()
            .contains("MAX_HEADER_BYTES"));
    }

    /// A subject with a scripted memory: it learns each planted fact from the turn that states it
    /// and loses whole categories at compaction and at resume.
    struct Fake {
        dir: tempfile::TempDir,
        known: HashSet<&'static str>,
        lose_on_compact: &'static [Category],
        lose_on_resume: &'static [Category],
        undos: usize,
        resumes: usize,
    }

    impl Fake {
        fn new(lose_on_compact: &'static [Category], lose_on_resume: &'static [Category]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            fixture::create(dir.path()).unwrap();
            Self {
                dir,
                known: HashSet::new(),
                lose_on_compact,
                lose_on_resume,
                undos: 0,
                resumes: 0,
            }
        }

        fn forget(&mut self, categories: &[Category]) {
            self.known.retain(|id| {
                let cat = all_probes().find(|p| p.id == *id).unwrap().category;
                !categories.contains(&cat)
            });
        }
    }

    impl Subject for Fake {
        async fn turn(&mut self, prompt: &str) -> TurnOutcome {
            let mut text = "ok".to_string();
            if let Some(turn) = SCENARIO.iter().find(|t| t.prompt == prompt) {
                self.known.extend(turn.probes.iter().map(|p| p.id));
            } else if let Some(question) = prompt.strip_prefix(ASK_PREFIX) {
                let probe = all_probes().find(|p| p.prompt == question).unwrap();
                text = if self.known.contains(probe.id) {
                    probe.good.into()
                } else {
                    "I do not recall.".into()
                };
            } else if let Some(probe) = BEHAVIOR_PROBES.iter().find(|p| p.prompt == prompt) {
                let rule = if probe.id == "obey_unwrap_rule" {
                    "unwrap_rule"
                } else {
                    "commit_prefix"
                };
                let body = if self.known.contains(rule) {
                    probe.good
                } else {
                    probe.bad
                };
                match probe.check {
                    Check::File { path, .. } => {
                        std::fs::write(self.dir.path().join(path), body).unwrap()
                    }
                    _ => {
                        std::fs::write(self.dir.path().join("note.txt"), body).unwrap();
                        crate::bench::run_git(self.dir.path(), &["add", "-A"]).unwrap();
                        crate::bench::run_git(self.dir.path(), &["commit", "-qm", body]).unwrap();
                    }
                }
            }
            TurnOutcome {
                text,
                tool_calls: 0,
                error: None,
            }
        }

        async fn compact(&mut self) -> Result<(usize, usize)> {
            self.forget(self.lose_on_compact);
            Ok((40, 8))
        }

        async fn undo(&mut self) -> Result<()> {
            self.undos += 1;
            Ok(())
        }

        async fn resume(&mut self) -> Result<()> {
            self.resumes += 1;
            self.forget(self.lose_on_resume);
            Ok(())
        }

        fn workdir(&self) -> &Path {
            self.dir.path()
        }

        fn usage(&self) -> UsageTotals {
            UsageTotals::default()
        }

        fn session_id(&self) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn the_driver_scores_a_fake_that_forgets_numbers_at_compaction_and_pending_at_resume() {
        let mut fake = Fake::new(&[Category::Number], &[Category::Pending]);
        let out = run(&mut fake, 1.0, false).await.unwrap();
        assert!(out.resumed);
        assert_eq!(fake.resumes, 1);
        assert_eq!(out.compactions.len(), 2);
        assert_eq!(out.compactions[0].messages_before, 40);
        assert_eq!(out.failed_turns, 0);
        assert_eq!(
            fake.undos,
            2 * all_probes().count(),
            "every probe is undone"
        );

        let a = score_phase(&out.probes, Phase::AfterCompaction);
        let b = score_phase(&out.probes, Phase::AfterResume);
        let total = all_probes().count();
        let numbers = all_probes()
            .filter(|p| p.category == Category::Number)
            .count();
        let pending = all_probes()
            .filter(|p| p.category == Category::Pending)
            .count();
        assert_eq!(
            a.total,
            Tally {
                passed: total - numbers,
                total
            }
        );
        assert_eq!(a.by_category[&Category::Number].passed, 0);
        assert_eq!(a.by_category[&Category::Pending].passed, pending);
        assert_eq!(
            b.total,
            Tally {
                passed: total - numbers - pending,
                total
            }
        );
        assert_eq!(
            a.by_category[&Category::Behavior],
            Tally {
                passed: 2,
                total: 2
            }
        );
        assert_eq!(a.by_zone[&Zone::Late].total, 3);
    }

    #[tokio::test]
    async fn skip_resume_runs_one_phase_and_a_forgetful_behavior_probe_fails() {
        let mut fake = Fake::new(&[Category::Constraint], &[]);
        let out = run(&mut fake, 0.0, true).await.unwrap();
        assert!(!out.resumed);
        assert_eq!(fake.resumes, 0);
        assert!(out.probes.iter().all(|p| p.phase == Phase::AfterCompaction));
        let a = score_phase(&out.probes, Phase::AfterCompaction);
        assert_eq!(
            a.by_category[&Category::Behavior].passed,
            0,
            "it forgot both rules"
        );
        assert_eq!(a.by_category[&Category::Constraint].passed, 0);
        assert_eq!(a.by_category[&Category::Decision].passed, 3);
    }
}
