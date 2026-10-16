//! Runs a scenario against a [`Subject`]: the scripted turns and compactions, then the probes
//! (before and after a resume), each probe independent of the others.
//!
//! The subject is a trait so the plumbing is testable with a scripted fake; the live one wraps a
//! real `Session` (`live.rs`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use regex::RegexBuilder;

use super::scenario::{Check, Probe, Scenario, Turn, Zone, ASK_PREFIX};
use super::score::{cap, CompactionLog, CompactionRecord, Phase, ProbeResult, UsageTotals};

pub(super) struct TurnOutcome {
    pub text: String,
    pub tool_calls: u64,
    pub error: Option<String>,
}

pub(super) trait Subject {
    async fn turn(&mut self, prompt: &str) -> TurnOutcome;
    /// Fold the transcript exactly as `/compact` does; returns (messages before, after).
    async fn compact(&mut self) -> Result<(usize, usize)>;
    /// Save the persisted session state (the store) as it is now.
    async fn checkpoint(&mut self) -> Result<()>;
    /// One turn on a session freshly resumed from the last [`checkpoint`](Self::checkpoint),
    /// discarded afterwards: the probe sees exactly what a resume gives, and nothing else.
    async fn turn_resumed(&mut self, prompt: &str) -> TurnOutcome;
    fn workdir(&self) -> &Path;
    fn usage(&self) -> UsageTotals;
    fn session_id(&self) -> Option<String>;
    /// Every compaction the session reported so far, automatic ones included.
    fn compaction_log(&self) -> CompactionLog;
}

pub(super) enum Step {
    Say(String),
    Compact(&'static str),
}

/// A run of consecutive filler zones: turns that state a fact are always kept; the others are
/// scaled to `round(count * scale)` (truncated below 1x, padded above it with `extras` and a
/// pass number).
fn scaled_run(run: &[Turn], scale: f64, extras: &[&'static str]) -> Vec<Step> {
    let filler = run.iter().filter(|t| t.probes.is_empty()).count();
    let want = (filler as f64 * scale.max(0.0)).round() as usize;
    let mut seen = 0;
    let mut steps = Vec::new();
    for t in run {
        if t.probes.is_empty() {
            seen += 1;
            if seen > want {
                continue;
            }
        }
        steps.push(Step::Say(t.prompt.to_string()));
    }
    for i in 0..want.saturating_sub(filler) {
        steps.push(Step::Say(format!(
            "{} (pass {})",
            extras[i % extras.len()],
            i / extras.len() + 2
        )));
    }
    steps
}

pub(super) fn plan(scenario: &Scenario, scale: f64) -> Vec<Step> {
    let turns = scenario.turns;
    let mut steps = Vec::new();
    let mut i = 0;
    while i < turns.len() {
        match turns[i].zone {
            Zone::Marker => {
                steps.push(Step::Compact(turns[i].prompt));
                i += 1;
            }
            Zone::Work | Zone::Wrap => {
                let start = i;
                while i < turns.len() && matches!(turns[i].zone, Zone::Work | Zone::Wrap) {
                    i += 1;
                }
                steps.extend(scaled_run(&turns[start..i], scale, scenario.extras));
            }
            Zone::Plant | Zone::Late => {
                steps.push(Step::Say(turns[i].prompt.to_string()));
                i += 1;
            }
        }
    }
    steps
}

pub(super) struct Outcome {
    pub scenario_turns: usize,
    pub failed_turns: usize,
    pub compactions: Vec<CompactionRecord>,
    pub probes: Vec<ProbeResult>,
    pub resumed: bool,
    /// How many of the session's compaction events happened before the first probe.
    pub scenario_events: usize,
}

pub(super) async fn run<S: Subject>(
    subject: &mut S,
    scenario: &Scenario,
    scale: f64,
    skip_resume: bool,
) -> Result<Outcome> {
    let steps = plan(scenario, scale);
    let total = steps.iter().filter(|s| matches!(s, Step::Say(_))).count();
    let mut out = Outcome {
        scenario_turns: total,
        failed_turns: 0,
        compactions: Vec::new(),
        probes: Vec::new(),
        resumed: false,
        scenario_events: 0,
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
    out.scenario_events = subject.compaction_log().events.len();
    eprintln!("{} compaction(s) during the scenario", out.scenario_events);
    // Both phases start from this exact state: the store and the workspace as the scenario left
    // them. The live-session phase below changes both, so it comes last-but-restored.
    subject
        .checkpoint()
        .await
        .context("checkpointing the store")?;
    let baseline = Snapshot::take(subject.workdir())?;
    probe_phase(
        subject,
        scenario,
        Phase::AfterCompaction,
        &baseline,
        &mut out.probes,
    )
    .await?;
    if !skip_resume {
        out.resumed = true;
        probe_phase(
            subject,
            scenario,
            Phase::AfterResume,
            &baseline,
            &mut out.probes,
        )
        .await?;
    }
    Ok(out)
}

async fn probe_phase<S: Subject>(
    subject: &mut S,
    scenario: &Scenario,
    phase: Phase,
    baseline: &Snapshot,
    results: &mut Vec<ProbeResult>,
) -> Result<()> {
    eprintln!("probing ({})", phase.label());
    baseline.restore()?;
    for probe in scenario.probes() {
        for result in run_probe(subject, scenario, probe, phase, baseline).await? {
            eprintln!(
                "  {} {} ({})",
                if result.passed { "pass" } else { "FAIL" },
                result.id,
                cap(&result.observed, 90)
            );
            results.push(result);
        }
    }
    Ok(())
}

/// One probe. `AfterResume` probes are isolated: each runs on its own session resumed from the
/// checkpoint. `AfterCompaction` probes run in sequence on the live session (there is no way to
/// rewind it: `/undo` after a compaction reloads the whole uncompacted history), so an earlier
/// answer can leak into a later one. The workspace goes back to `baseline` after every probe.
async fn run_probe<S: Subject>(
    subject: &mut S,
    scenario: &Scenario,
    probe: &Probe,
    phase: Phase,
    baseline: &Snapshot,
) -> Result<Vec<ProbeResult>> {
    let workdir = subject.workdir().to_path_buf();
    let head_before = head(&workdir).ok();
    let prompt = match probe.check {
        Check::Answer(_) | Check::WorkState { .. } => format!("{ASK_PREFIX}{}", probe.prompt),
        _ => probe.prompt.to_string(),
    };
    let turn = match phase {
        Phase::AfterCompaction => subject.turn(&prompt).await,
        Phase::AfterResume => subject.turn_resumed(&prompt).await,
    };
    let verdicts = evaluate(&probe.check, &turn.text, &workdir, head_before.as_deref());
    baseline.restore()?;
    Ok(verdicts
        .into_iter()
        .map(|v| ProbeResult {
            id: format!("{}{}", probe.id, v.suffix),
            category: probe.category,
            zone: scenario.zone_of(probe.id),
            phase,
            passed: v.passed,
            observed: v.observed,
            tool_calls: turn.tool_calls,
            error: turn.error.clone(),
        })
        .collect())
}

pub(super) struct Verdict {
    /// Appended to the probe id when one probe yields several results.
    pub suffix: &'static str,
    pub passed: bool,
    pub observed: String,
}

fn one(passed: bool, observed: String) -> Vec<Verdict> {
    vec![Verdict {
        suffix: "",
        passed,
        observed,
    }]
}

pub(super) fn evaluate(
    check: &Check,
    reply: &str,
    workdir: &Path,
    head_before: Option<&str>,
) -> Vec<Verdict> {
    match check {
        Check::Answer(matcher) => one(matcher.accepts(reply), cap(reply, 200)),
        Check::File { path, matcher } => match std::fs::read_to_string(workdir.join(path)) {
            Ok(body) => {
                let ok = matcher.accepts(&body);
                let verdict = if ok {
                    "file ok"
                } else {
                    "file violates the rule"
                };
                one(ok, format!("{verdict} | reply: {}", cap(reply, 120)))
            }
            Err(e) => one(false, format!("cannot read {path}: {e}")),
        },
        Check::CommitSubject(matcher) => {
            let now = head(workdir).ok();
            if now.is_none() || now.as_deref() == head_before {
                return one(false, format!("no new commit | reply: {}", cap(reply, 120)));
            }
            let subject = crate::bench::run_git(workdir, &["log", "-1", "--format=%s"])
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            one(matcher.accepts(&subject), format!("commit: {subject}"))
        }
        Check::WorkState { pending } => {
            let changed = changed_files(workdir);
            let missing: Vec<&String> = changed
                .iter()
                .filter(|f| !mentions_file(reply, f))
                .collect();
            let files_ok = !changed.is_empty() && missing.is_empty();
            let files_note = if changed.is_empty() {
                "no changed files to check".to_string()
            } else if missing.is_empty() {
                format!("all {} changed file(s) named", changed.len())
            } else {
                format!(
                    "missing {} of {}: {}",
                    missing.len(),
                    changed.len(),
                    missing
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let lost: Vec<usize> = pending
                .iter()
                .enumerate()
                .filter(|(_, m)| !m.accepts(reply))
                .map(|(i, _)| i)
                .collect();
            let pending_note = if lost.is_empty() {
                format!("all {} pending item(s) named", pending.len())
            } else {
                format!("pending item(s) {lost:?} not named of {}", pending.len())
            };
            vec![
                Verdict {
                    suffix: "_files",
                    passed: files_ok,
                    observed: format!("{files_note} | reply: {}", cap(reply, 140)),
                },
                Verdict {
                    suffix: "_pending",
                    passed: lost.is_empty(),
                    observed: format!("{pending_note} | reply: {}", cap(reply, 140)),
                },
            ]
        }
    }
}

/// Files that differ from the fixture's baseline commit: committed edits (the model is free to
/// commit mid-session), uncommitted edits and new files. Forge's own state dir and the
/// `Cargo.lock` a `cargo` run leaves behind are not the model's work.
pub(super) fn changed_files(workdir: &Path) -> Vec<String> {
    let git = |args: &[&str]| crate::bench::run_git(workdir, args).unwrap_or_default();
    let root = git(&["rev-list", "--max-parents=0", "HEAD"]);
    let root = root.lines().next().unwrap_or("HEAD").to_string();
    let tracked = git(&["diff", "--name-only", &root]);
    let untracked = git(&["ls-files", "--others", "--exclude-standard"]);
    let mut files: Vec<String> = tracked
        .lines()
        .chain(untracked.lines())
        .map(|l| l.trim().trim_matches('"').to_string())
        .filter(|p| {
            !p.is_empty()
                && !p.starts_with(".forge/")
                && !p.starts_with("target/")
                && p != "Cargo.lock"
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Whether the reply names `path`: its file name is enough, so `limits.rs` counts for
/// `src/limits.rs`.
fn mentions_file(reply: &str, path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    RegexBuilder::new(&regex::escape(name))
        .case_insensitive(true)
        .build()
        .is_ok_and(|re| re.is_match(reply))
}

fn head(workdir: &Path) -> Result<String> {
    crate::bench::run_git(workdir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

/// A copy of the workspace (git state included), restorable any number of times.
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

    fn restore(&self) -> Result<()> {
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
    use super::super::scenario::{Category, HARD, STANDARD};
    use super::super::score::{score_phase, CompactionEvent, Tally};
    use super::*;

    const BOTH: [&Scenario; 2] = [&STANDARD, &HARD];

    fn say_count(steps: &[Step]) -> usize {
        steps.iter().filter(|s| matches!(s, Step::Say(_))).count()
    }

    fn position(steps: &[Step], needle: &str) -> usize {
        steps
            .iter()
            .position(|s| matches!(s, Step::Say(p) if p.contains(needle)))
            .unwrap_or_else(|| panic!("no turn contains {needle}"))
    }

    #[test]
    fn plan_orders_zones_around_the_two_manual_compactions() {
        for sc in BOTH {
            let steps = plan(sc, 1.0);
            let compacts: Vec<usize> = steps
                .iter()
                .enumerate()
                .filter(|(_, s)| matches!(s, Step::Compact(_)))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(compacts.len(), 2, "{}", sc.name);
            assert_eq!(compacts[1], steps.len() - 1, "the last compaction is last");
            let says = say_count(&steps);
            let range = if sc.name == "hard" { 35..=45 } else { 15..=25 };
            assert!(range.contains(&says), "{} scenario size: {says}", sc.name);
        }
        let steps = plan(&STANDARD, 1.0);
        let mid = steps
            .iter()
            .position(|s| matches!(s, Step::Compact(_)))
            .unwrap();
        assert!(position(&steps, "hand-rolled") < mid);
        assert!(position(&steps, "# Errors") > mid);
        let steps = plan(&HARD, 1.0);
        let mid = steps
            .iter()
            .position(|s| matches!(s, Step::Compact(_)))
            .unwrap();
        assert!(position(&steps, "house rules") < mid);
        assert!(
            position(&steps, "use tabs") > mid,
            "the override comes after compaction #1"
        );
    }

    #[test]
    fn turns_scale_truncates_and_pads_filler_but_never_the_fact_turns() {
        for sc in BOTH {
            let filler = sc
                .turns
                .iter()
                .filter(|t| matches!(t.zone, Zone::Work | Zone::Wrap) && t.probes.is_empty())
                .count();
            let fixed = sc.turns.iter().filter(|t| t.zone != Zone::Marker).count() - filler;
            let count = |scale: f64| say_count(&plan(sc, scale));
            assert_eq!(count(0.0), fixed, "{}", sc.name);
            assert_eq!(count(1.0), fixed + filler, "{}", sc.name);
            assert_eq!(count(2.0), fixed + 2 * filler, "{}", sc.name);
            assert!(plan(sc, 3.0)
                .iter()
                .any(|s| matches!(s, Step::Say(p) if p.ends_with("(pass 2)"))));
        }
        let steps = plan(&HARD, 0.0);
        position(&steps, "data/region_table.csv");
        position(&steps, "run_checks.sh");
    }

    #[test]
    fn probe_ids_and_questions_are_unique_within_a_scenario() {
        for sc in BOTH {
            let mut ids = HashSet::new();
            let mut prompts = HashSet::new();
            for p in sc.probes() {
                assert!(ids.insert(p.id), "duplicate probe id {}", p.id);
                assert!(prompts.insert(p.prompt), "duplicate probe prompt {}", p.id);
            }
            assert!(sc
                .behavior
                .iter()
                .all(|p| !matches!(p.check, Check::Answer(_))));
            assert!(Scenario::by_name(sc.name).is_some());
        }
        assert!(Scenario::by_name("nope").is_none());
    }

    #[test]
    fn evaluate_checks_files_and_commits_in_a_real_repo() {
        let tmp = tempfile::tempdir().unwrap();
        fixture::create(tmp.path(), STANDARD.fixture).unwrap();
        let wd = tmp.path();
        let before = head(wd).unwrap();
        let find = |id: &str| STANDARD.behavior.iter().find(|p| p.id == id).unwrap();
        let (unwrap_probe, commit_probe) = (find("obey_unwrap_rule"), find("obey_commit_prefix"));
        let passed = |p: &Probe| evaluate(&p.check, "done", wd, Some(&before))[0].passed;

        assert!(!passed(unwrap_probe), "the function does not exist yet");
        std::fs::write(wd.join("src/parser.rs"), unwrap_probe.bad).unwrap();
        assert!(!passed(unwrap_probe));
        std::fs::write(wd.join("src/parser.rs"), unwrap_probe.good).unwrap();
        assert!(passed(unwrap_probe));

        assert!(!passed(commit_probe), "no new commit");
        crate::bench::run_git(wd, &["commit", "-qam", "fix: no prefix"]).unwrap();
        assert!(!passed(commit_probe), "wrong prefix");
        std::fs::write(wd.join("src/limits.rs"), "// changed\n").unwrap();
        crate::bench::run_git(wd, &["commit", "-qam", "JIRA-123: right prefix"]).unwrap();
        assert!(passed(commit_probe));
    }

    #[test]
    fn the_work_state_probe_scores_the_reply_against_the_real_git_state() {
        let tmp = tempfile::tempdir().unwrap();
        fixture::create(tmp.path(), HARD.fixture).unwrap();
        let wd = tmp.path();
        let probe = HARD.probes().find(|p| p.id == "work_state").unwrap();
        let verdicts = |reply: &str| evaluate(&probe.check, reply, wd, None);

        assert!(
            !verdicts("nothing")[0].passed,
            "an untouched repo has nothing to name"
        );
        assert!(changed_files(wd).is_empty());
        std::fs::write(wd.join("src/limits.rs"), "// edit\n").unwrap();
        crate::bench::run_git(
            wd,
            &["commit", "-qam", "JIRA-123: the model commits its own work"],
        )
        .unwrap();
        std::fs::write(wd.join("Cargo.lock"), "# build byproduct\n").unwrap();
        std::fs::create_dir_all(wd.join("tests")).unwrap();
        std::fs::write(wd.join("tests/new_case.rs"), "// new\n").unwrap();
        std::fs::create_dir_all(wd.join(".forge/checkpoints")).unwrap();
        std::fs::write(wd.join(".forge/checkpoints/x"), "state").unwrap();
        let mut changed = changed_files(wd);
        changed.sort();
        assert_eq!(
            changed,
            ["src/limits.rs", "tests/new_case.rs"],
            "untracked files listed, .forge hidden"
        );

        let v = verdicts(probe.good);
        assert_eq!(v.len(), 2);
        assert!(
            !v[0].passed,
            "good sample names limits.rs but not new_case.rs"
        );
        assert!(
            v[0].observed.contains("tests/new_case.rs"),
            "{}",
            v[0].observed
        );
        assert!(v[1].passed, "all three pending items named");

        let v = verdicts(&format!("{} Also tests/NEW_CASE.rs.", probe.good));
        assert!(
            v[0].passed,
            "file names match case-insensitively by basename"
        );
        let v = verdicts("Changed src/limits.rs and tests/new_case.rs. Left: the regression test for empty headers.");
        assert!(v[0].passed);
        assert!(!v[1].passed, "two of the three pending items are missing");
        assert!(v[1].observed.contains("[1, 2]"), "{}", v[1].observed);
    }

    #[test]
    fn a_snapshot_restores_edits_new_files_and_commits() {
        let tmp = tempfile::tempdir().unwrap();
        fixture::create(tmp.path(), STANDARD.fixture).unwrap();
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
        scenario: &'static Scenario,
        dir: tempfile::TempDir,
        known: HashSet<&'static str>,
        lose_on_compact: &'static [Category],
        lose_on_resume: &'static [Category],
        events: Vec<CompactionEvent>,
        touched: bool,
        checkpoint_known: HashSet<&'static str>,
        checkpoints: usize,
        resumed_turns: usize,
    }

    impl Fake {
        fn new(
            scenario: &'static Scenario,
            lose_on_compact: &'static [Category],
            lose_on_resume: &'static [Category],
        ) -> Self {
            let dir = tempfile::tempdir().unwrap();
            fixture::create(dir.path(), scenario.fixture).unwrap();
            Self {
                scenario,
                dir,
                // AGENTS.md is read at session start, before any scripted turn.
                known: HashSet::from(["agents_md"]),
                lose_on_compact,
                lose_on_resume,
                events: Vec::new(),
                touched: false,
                checkpoint_known: HashSet::new(),
                checkpoints: 0,
                resumed_turns: 0,
            }
        }

        fn forget(&mut self, categories: &[Category]) {
            let sc = self.scenario;
            self.known.retain(|id| {
                if *id == "agents_md" {
                    return !categories.contains(&Category::Constraint);
                }
                let cat = sc.probes().find(|p| p.id == *id).unwrap().category;
                !categories.contains(&cat)
            });
        }

        fn work_state_reply(&self) -> String {
            let mut parts = vec![changed_files(self.dir.path()).join(", ")];
            for (id, phrase) in [
                ("pending_test", "regression test for the empty-header case"),
                ("pending_version", "bump to 0.4.2"),
                ("pending_changelog", "changelog section"),
            ] {
                if self.known.contains(id) {
                    parts.push(phrase.to_string());
                }
            }
            parts.join("; ")
        }
    }

    impl Subject for Fake {
        async fn turn(&mut self, prompt: &str) -> TurnOutcome {
            let sc = self.scenario;
            let mut text = "ok".to_string();
            if let Some(turn) = sc.turns.iter().find(|t| t.prompt == prompt) {
                self.known.extend(turn.probes.iter().map(|p| p.id));
                if !self.touched {
                    self.touched = true;
                    let path = self.dir.path().join("src/limits.rs");
                    let body = std::fs::read_to_string(&path).unwrap();
                    std::fs::write(&path, format!("{body}// touched\n")).unwrap();
                }
            } else if let Some(question) = prompt.strip_prefix(ASK_PREFIX) {
                let probe = sc.probes().find(|p| p.prompt == question).unwrap();
                text = if matches!(probe.check, Check::WorkState { .. }) {
                    self.work_state_reply()
                } else if self.known.contains(probe.id) {
                    probe.good.into()
                } else {
                    "I do not recall.".into()
                };
            } else if let Some(probe) = sc.behavior.iter().find(|p| p.prompt == prompt) {
                let rule = match probe.id {
                    "obey_unwrap_rule" => "unwrap_rule",
                    "obey_commit_prefix" => "commit_prefix",
                    "obey_agents_doc_rule" => "agents_md",
                    "obey_latest_indent" => "indent_override",
                    other => panic!("fake does not know {other}"),
                };
                let body = if self.known.contains(rule) {
                    probe.good
                } else {
                    probe.bad
                };
                match probe.check {
                    Check::File { path, .. } => {
                        let file = self.dir.path().join(path);
                        let old = std::fs::read_to_string(&file).unwrap_or_default();
                        std::fs::write(file, format!("{old}\n{body}")).unwrap();
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
            self.events.push(CompactionEvent {
                auto: false,
                messages_before: 40,
                messages_after: 8,
                summary_model: Some("fake".into()),
            });
            Ok((40, 8))
        }

        async fn checkpoint(&mut self) -> Result<()> {
            self.checkpoints += 1;
            self.checkpoint_known = self.known.clone();
            Ok(())
        }

        async fn turn_resumed(&mut self, prompt: &str) -> TurnOutcome {
            self.resumed_turns += 1;
            let live = std::mem::replace(&mut self.known, self.checkpoint_known.clone());
            self.forget(self.lose_on_resume);
            let out = self.turn(prompt).await;
            self.known = live;
            out
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

        fn compaction_log(&self) -> CompactionLog {
            CompactionLog {
                events: self.events.clone(),
                thin_summary_warnings: 0,
            }
        }
    }

    fn results_per_phase(sc: &Scenario) -> usize {
        sc.probes()
            .map(|p| {
                if matches!(p.check, Check::WorkState { .. }) {
                    2
                } else {
                    1
                }
            })
            .sum()
    }

    fn of(sc: &Scenario, category: Category) -> usize {
        sc.probes()
            .filter(|p| p.category == category)
            .map(|p| {
                if matches!(p.check, Check::WorkState { .. }) {
                    2
                } else {
                    1
                }
            })
            .sum()
    }

    #[tokio::test]
    async fn the_driver_scores_a_fake_that_forgets_numbers_at_compaction_and_pending_at_resume() {
        let mut fake = Fake::new(&STANDARD, &[Category::Number], &[Category::Pending]);
        let out = run(&mut fake, &STANDARD, 1.0, false).await.unwrap();
        assert!(out.resumed);
        assert_eq!(fake.checkpoints, 1);
        assert_eq!(
            fake.resumed_turns,
            STANDARD.probes().count(),
            "every resumed probe gets its own fresh session"
        );
        assert_eq!(out.compactions.len(), 2);
        assert_eq!(out.compactions[0].messages_before, 40);
        assert_eq!(out.scenario_events, 2);
        assert_eq!(out.failed_turns, 0);

        let a = score_phase(&out.probes, Phase::AfterCompaction);
        let b = score_phase(&out.probes, Phase::AfterResume);
        let total = results_per_phase(&STANDARD);
        let numbers = of(&STANDARD, Category::Number);
        let pending = of(&STANDARD, Category::Pending);
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
        let mut fake = Fake::new(&STANDARD, &[Category::Constraint], &[]);
        let out = run(&mut fake, &STANDARD, 0.0, true).await.unwrap();
        assert!(!out.resumed);
        assert_eq!(fake.resumed_turns, 0);
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

    #[tokio::test]
    async fn the_hard_scenario_scores_tool_results_work_state_tasks_and_the_agents_md_rule() {
        let mut fake = Fake::new(
            &HARD,
            &[Category::ToolResult, Category::Number],
            &[Category::TaskList],
        );
        let out = run(&mut fake, &HARD, 0.0, false).await.unwrap();
        let a = score_phase(&out.probes, Phase::AfterCompaction);
        let b = score_phase(&out.probes, Phase::AfterResume);
        let total = results_per_phase(&HARD);
        assert_eq!(a.total.total, total);
        assert_eq!(a.by_category[&Category::ToolResult].passed, 0);
        assert_eq!(a.by_category[&Category::Number].passed, 0);
        assert_eq!(
            a.by_category[&Category::WorkState],
            Tally {
                passed: 2,
                total: 2
            },
            "the work-state reply names the changed file and the surviving pending items"
        );
        assert_eq!(a.by_category[&Category::TaskList].passed, 1);
        assert_eq!(b.by_category[&Category::TaskList].passed, 0);
        assert_eq!(
            a.by_category[&Category::Behavior],
            Tally {
                passed: 3,
                total: 3
            }
        );
        let lost = of(&HARD, Category::ToolResult) + of(&HARD, Category::Number);
        assert_eq!(a.total.passed, total - lost);

        let mut forgetful = Fake::new(&HARD, &[Category::Constraint, Category::Correction], &[]);
        let out = run(&mut forgetful, &HARD, 0.0, true).await.unwrap();
        let a = score_phase(&out.probes, Phase::AfterCompaction);
        assert_eq!(
            a.by_category[&Category::Behavior],
            Tally {
                passed: 0,
                total: 3
            },
            "no AGENTS.md rule, no latest indent, no commit prefix"
        );
    }
}
