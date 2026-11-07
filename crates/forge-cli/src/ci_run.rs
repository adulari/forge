//! Run the local CI plan (docs/features/ci-parity.md): the engine behind `forge ci run`, the
//! `/goal` finish gate and the pre-push check of ship and the merge queue.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use forge_config::ci_parity::plan::{build_plan, CiPlan, CiStep, PlanContext};
use forge_config::ci_parity::report::{load_report, save_report, CiReport, StepResult, StepStatus};
use forge_config::ci_parity::settings::{apply_overrides, load_settings, CiSettings, GateMode};
use sha2::Digest as _;

pub(crate) mod live;

const TAIL_CHARS: usize = 3000;

/// Set to `off` to disable every automatic use of the plan (finish gate, pre-push) at once.
pub(crate) const KILL_SWITCH_ENV: &str = "FORGE_CI_GATE";

#[derive(Default)]
pub(crate) struct RunOptions {
    pub(crate) keep_going: bool,
    pub(crate) fresh: bool,
    /// Run only steps whose key contains this text.
    pub(crate) only: Option<String>,
}

pub(crate) enum Event<'a> {
    Start {
        index: usize,
        total: usize,
        step: &'a CiStep,
    },
    Done {
        result: &'a StepResult,
    },
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Files that differ from the base branch (committed, staged, unstaged and untracked).
fn changed_files(root: &Path) -> Option<Vec<String>> {
    let base = ["origin/main", "origin/master"]
        .iter()
        .find_map(|b| git(root, &["merge-base", "HEAD", b]))?;
    let mut files: Vec<String> = git(root, &["diff", "--name-only", base.trim()])?
        .lines()
        .map(str::to_string)
        .collect();
    if let Some(untracked) = git(root, &["ls-files", "--others", "--exclude-standard"]) {
        files.extend(untracked.lines().map(str::to_string));
    }
    Some(files)
}

pub(crate) fn runner_temp(root: &Path) -> PathBuf {
    let tag = hex::encode(sha2::Sha256::digest(root.to_string_lossy().as_bytes()));
    std::env::temp_dir().join(format!("forge-ci-{}", &tag[..12]))
}

/// The plan for `root` with `.forge/ci.toml` overrides applied.
pub(crate) fn make_plan(root: &Path) -> (CiPlan, CiSettings) {
    let settings = load_settings(root);
    let ctx = PlanContext {
        runner_temp: runner_temp(root),
        workspace: root.to_path_buf(),
        changed_files: changed_files(root),
    };
    let mut plan = build_plan(root, &ctx);
    apply_overrides(&mut plan, &settings);
    (plan, settings)
}

/// Hash of everything that can change a check's result: HEAD plus the uncommitted state.
fn tree_fingerprint(root: &Path) -> String {
    if git(root, &["rev-parse", "HEAD"]).is_none() {
        // Without git there is no way to tell that the tree is unchanged, so nothing is reused.
        return format!(
            "untracked-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
    }
    let mut text = String::new();
    for args in [
        &["rev-parse", "HEAD"][..],
        &["status", "--porcelain"],
        &["diff", "HEAD"],
    ] {
        text.push_str(&git(root, args).unwrap_or_default());
        text.push('\n');
    }
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

fn tail(text: &str) -> String {
    let total = text.chars().count();
    if total <= TAIL_CHARS {
        return text.trim_end().to_string();
    }
    let kept: String = text.chars().skip(total - TAIL_CHARS).collect();
    format!(
        "...[truncated {} earlier chars]...\n{}",
        total - TAIL_CHARS,
        kept.trim_end()
    )
}

fn shell_args(shell: Option<&str>) -> (&'static str, &'static [&'static str]) {
    match shell {
        Some("sh") => ("sh", &["-e"]),
        Some("bash") => ("bash", &["--noprofile", "--norc", "-eo", "pipefail"]),
        _ => ("bash", &["-e"]),
    }
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Run one step to completion or timeout; the combined output tail comes back with the status.
fn run_step(
    root: &Path,
    plan: &CiPlan,
    step: &CiStep,
    index: usize,
    limit: Duration,
) -> (StepStatus, String) {
    let temp = PathBuf::from(&plan.runner_temp);
    if let Err(e) = std::fs::create_dir_all(&temp) {
        return (
            StepStatus::Fail,
            format!("cannot create {}: {e}", temp.display()),
        );
    }
    let script = temp.join(format!("step-{index}.sh"));
    let log_path = temp.join(format!("step-{index}.log"));
    let (log, log_err) =
        match std::fs::File::create(&log_path).and_then(|f| Ok((f.try_clone()?, f))) {
            Ok((a, b)) => (a, b),
            Err(e) => return (StepStatus::Fail, format!("cannot create the step log: {e}")),
        };
    if let Err(e) = std::fs::write(&script, &step.run) {
        return (
            StepStatus::Fail,
            format!("cannot write the step script: {e}"),
        );
    }
    let (program, flags) = shell_args(step.shell.as_deref());
    let dir = step
        .cwd
        .as_deref()
        .map_or_else(|| root.to_path_buf(), |c| root.join(c));
    let mut cmd = Command::new(program);
    cmd.args(flags)
        .arg(&script)
        .current_dir(dir)
        .env("CI", "true")
        .env("GITHUB_WORKSPACE", root)
        .env("RUNNER_TEMP", &temp)
        .env("RUNNER_OS", "Linux")
        .envs(&step.env)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (
                StepStatus::Fail,
                format!("could not start `{program}`: {e}"),
            )
        }
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if started.elapsed() >= limit => break None,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return (StepStatus::Fail, format!("waiting failed: {e}")),
        }
    };
    let output = std::fs::read(&log_path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    match status {
        Some(s) if s.success() => (StepStatus::Pass, String::new()),
        Some(s) if s.code() == Some(127) || output.contains("no such command") => (
            StepStatus::Skipped,
            format!("not installed here: {}", tail(&output)),
        ),
        Some(s) => (StepStatus::Fail, format!("{}\n[exit: {s}]", tail(&output))),
        None => {
            kill_tree(&mut child);
            (
                StepStatus::Timeout,
                format!(
                    "{}\n[timed out after {}s and killed]",
                    tail(&output),
                    limit.as_secs()
                ),
            )
        }
    }
}

fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pg = child.id() as i32;
        unsafe { libc::kill(-pg, libc::SIGTERM) };
        std::thread::sleep(Duration::from_secs(2));
        unsafe { libc::kill(-pg, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn skipped_lines(plan: &CiPlan) -> Vec<String> {
    plan.skips.iter().map(skip_line).collect()
}

pub(crate) fn skip_line(s: &forge_config::ci_parity::plan::CiSkip) -> String {
    let at = [
        Some(s.workflow.as_str()),
        s.job.as_deref(),
        s.step.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" / ");
    format!("{at}: {}", s.reason)
}

/// Run the plan in order. Fail-fast unless `keep_going`; steps that passed on this exact tree
/// last time are not run again unless `fresh`. The report is saved for every surface.
pub(crate) fn run_plan(
    root: &Path,
    plan: &CiPlan,
    settings: &CiSettings,
    opts: &RunOptions,
    on_event: &mut dyn FnMut(Event<'_>),
) -> CiReport {
    let tree = tree_fingerprint(root);
    let previous = load_report(root).filter(|r| r.tree == tree && !opts.fresh);
    let steps: Vec<&CiStep> = plan
        .steps
        .iter()
        .filter(|s| opts.only.as_deref().is_none_or(|o| s.key.contains(o)))
        .collect();
    let mut report = CiReport {
        started_unix: now_unix(),
        tree,
        skipped: skipped_lines(plan),
        ..CiReport::default()
    };
    let begun = Instant::now();
    let budget = Duration::from_secs(settings.total_budget_secs);
    let mut halted = false;
    let tracked = opts.only.is_none();
    if tracked {
        live::begin(root, plan.steps.len(), plan.skips.len());
    }
    for (i, step) in steps.iter().enumerate() {
        let command = step.run.trim().to_string();
        let mut result = StepResult {
            key: step.key.clone(),
            name: step.name.clone(),
            command,
            status: StepStatus::NotRun,
            secs: 0,
            tail: String::new(),
        };
        if halted {
            on_event(Event::Done { result: &result });
            if tracked {
                live::step_done(root, &result);
            }
            report.results.push(result);
            continue;
        }
        let cached = previous.as_ref().is_some_and(|p| {
            p.results.iter().any(|r| {
                r.key == step.key
                    && r.command == result.command
                    && matches!(r.status, StepStatus::Pass | StepStatus::Cached)
            })
        });
        on_event(Event::Start {
            index: i + 1,
            total: steps.len(),
            step,
        });
        if tracked {
            live::step_started(root, &step.name, i + 1, steps.len());
        }
        if cached {
            result.status = StepStatus::Cached;
        } else if begun.elapsed() >= budget {
            report.stopped = Some(format!(
                "total budget of {} min spent",
                settings.total_budget_secs / 60
            ));
            halted = true;
        } else {
            let limit =
                Duration::from_secs(step.timeout_secs.unwrap_or(settings.step_timeout_secs))
                    .min(budget.saturating_sub(begun.elapsed()));
            let at = Instant::now();
            let (status, out) = run_step(root, plan, step, i, limit);
            result.secs = at.elapsed().as_secs();
            result.status = status;
            result.tail = out;
            if status == StepStatus::Skipped {
                // Nothing ran: leave the status as Skipped, never as a pass or a failure.
            } else if status != StepStatus::Pass && step.allow_failure {
                result.status = StepStatus::Pass;
                result.tail = format!("failed but continue-on-error is set\n{}", result.tail);
            } else if status != StepStatus::Pass && !opts.keep_going {
                report.stopped = Some(format!("`{}` failed", step.name));
                halted = true;
            }
        }
        on_event(Event::Done { result: &result });
        if tracked {
            live::step_done(root, &result);
        }
        report.results.push(result);
    }
    report.finished_unix = now_unix();
    if tracked {
        let _ = save_report(root, &report);
        live::finished(root, &report);
    }
    report
}

/// The step's environment as `K=V` pairs, without the cargo output-shaping noise.
fn shown_env(step: &CiStep) -> Vec<String> {
    step.env
        .iter()
        .filter(|(k, _)| !k.starts_with("CARGO_TERM") && !k.starts_with("CARGO_PROFILE"))
        .map(|(k, v)| format!("{k}={v}"))
        .collect()
}

/// What the model (or the owner) reads when the plan fails: step, command, env, output tail.
pub(crate) fn failure_text(plan: &CiPlan, report: &CiReport) -> String {
    let Some(f) = report.first_failure() else {
        return report
            .stopped
            .clone()
            .unwrap_or_else(|| "the CI plan did not complete".to_string());
    };
    let env = plan
        .steps
        .iter()
        .find(|s| s.key == f.key)
        .map(|s| shown_env(s).join(" "))
        .unwrap_or_default();
    format!(
        "Local CI step failed: {}\nStep key: {}\nCommand: {}\nEnvironment: {}\nOutput (tail):\n{}\n\
         Reproduce with `forge ci run --step \"{}\"`. This is the same check the repository's CI \
         runs, with its environment applied.",
        f.name,
        f.key,
        f.command.lines().next().unwrap_or(""),
        if env.is_empty() { "(default)" } else { &env },
        f.tail,
        f.key
    )
}

pub(crate) enum Check {
    /// Nothing to run (gate off, no workflows, no runnable steps); the reason says why.
    Skipped(String),
    Passed(String),
    Failed(String),
}

/// The automatic check used before a push and as the `/goal` finish gate. `via` names what is
/// waiting on it (`goal` or `push`), so a failing step can be shown as what blocked it.
pub(crate) fn automatic_check(root: &Path, via: &'static str) -> Check {
    if std::env::var(KILL_SWITCH_ENV).is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        return Check::Skipped(format!("{KILL_SWITCH_ENV}=off"));
    }
    let (plan, settings) = make_plan(root);
    if settings.gate == GateMode::Off {
        return Check::Skipped("[ci] gate = \"off\"".to_string());
    }
    if plan.steps.is_empty() {
        return Check::Skipped("no runnable workflow steps".to_string());
    }
    let report = run_plan(root, &plan, &settings, &RunOptions::default(), &mut |_| {});
    if report.stopped.is_none()
        && !report.results.is_empty()
        && report
            .results
            .iter()
            .all(|r| r.status == StepStatus::Skipped)
    {
        // Nothing ran, so nothing failed: a missing tool is not a reason to refuse the push.
        let commands: Vec<String> = report
            .results
            .iter()
            .map(|r| format!("`{}`", r.command.lines().next().unwrap_or("")))
            .collect();
        return Check::Skipped(format!(
            "every runnable step is not installed here: {}",
            commands.join(", ")
        ));
    }
    if report.ok() {
        let (pass, _, _) = report.counts();
        Check::Passed(format!("{pass} local CI steps passed"))
    } else {
        live::blocked(root, via, &report);
        Check::Failed(failure_text(&plan, &report))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ViewState {
    Pending,
    Pass,
    Cached,
    Fail,
    NotRun,
    Skip,
}

/// One line of the plan as every surface draws it: a step with its last result, or a skip.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ViewRow {
    pub(crate) state: ViewState,
    /// `workflow::job::step`, what `forge ci run --step` and `.forge/ci.toml` match; empty for a
    /// skip.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) key: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) job: String,
    /// The step's whole script (`detail` is its first line).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) command: String,
    /// The environment the step runs with, as `K=V`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) env: Vec<String>,
    /// The tail of the output of a step that failed or timed out.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) tail: String,
    pub(crate) name: String,
    /// The command for a step, the reason for a skip.
    pub(crate) detail: String,
    pub(crate) secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct CiView {
    pub(crate) rows: Vec<ViewRow>,
    /// `Last run 3 min ago: 12 passed, 1 failed, 0 not run`.
    pub(crate) summary: Option<String>,
    pub(crate) noops: usize,
}

pub(crate) fn view_for(plan: &CiPlan, report: Option<&CiReport>) -> CiView {
    CiView {
        rows: rows_for(plan, report),
        summary: summary_for(report),
        noops: plan.noops,
    }
}

/// Plan steps first (with the last run's result if there is one), then the skips.
pub(crate) fn rows_for(plan: &CiPlan, report: Option<&CiReport>) -> Vec<ViewRow> {
    let mut rows: Vec<ViewRow> = plan
        .steps
        .iter()
        .map(|step| {
            let result = report.and_then(|r| r.results.iter().find(|x| x.key == step.key));
            let state = match result.map(|r| r.status) {
                None => ViewState::Pending,
                Some(StepStatus::Pass) => ViewState::Pass,
                Some(StepStatus::Cached) => ViewState::Cached,
                Some(StepStatus::Fail | StepStatus::Timeout) => ViewState::Fail,
                Some(StepStatus::NotRun | StepStatus::Skipped) => ViewState::NotRun,
            };
            let first = step
                .run
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            ViewRow {
                state,
                key: step.key.clone(),
                job: step.job.clone(),
                command: step.run.trim().to_string(),
                env: shown_env(step),
                tail: result
                    .filter(|_| state == ViewState::Fail)
                    .map(|r| r.tail.clone())
                    .unwrap_or_default(),
                name: format!("{} / {}", step.job, step.name),
                detail: first.to_string(),
                secs: result.map_or(0, |r| r.secs),
            }
        })
        .collect();
    rows.extend(plan.skips.iter().map(|s| {
        let at: Vec<&str> = [s.job.as_deref(), s.step.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        ViewRow {
            state: ViewState::Skip,
            key: String::new(),
            job: s.job.clone().unwrap_or_default(),
            command: String::new(),
            env: Vec::new(),
            tail: String::new(),
            name: if at.is_empty() {
                s.workflow.clone()
            } else {
                at.join(" / ")
            },
            detail: s.reason.clone(),
            secs: 0,
        }
    }));
    rows
}

pub(crate) fn summary_for(report: Option<&CiReport>) -> Option<String> {
    let report = report?;
    let (pass, fail, rest) = report.counts();
    let ago = chrono::Utc::now()
        .timestamp()
        .saturating_sub(report.finished_unix)
        .max(0);
    let when = match ago {
        0..=89 => "just now".to_string(),
        90..=5399 => format!("{} min ago", ago / 60),
        _ => format!("{} h ago", ago / 3600),
    };
    let mut text = format!("Last run {when}: {pass} passed, {fail} failed, {rest} not run");
    if let Some(why) = &report.stopped {
        text.push_str(&format!(" ({why})"));
    }
    Some(text)
}

/// `Some(failure text)` when the local CI plan fails and the push must not happen. Blocking.
pub(crate) fn push_blocker(root: &Path) -> Option<String> {
    match automatic_check(root, "push") {
        Check::Failed(text) => Some(text),
        Check::Passed(_) | Check::Skipped(_) => None,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod view_tests;
