//! What the local CI plan is doing right now, per repository, for the remote snapshot: a run in
//! flight, the last run's result per step, and the step holding a goal or a push back. Runs
//! update it as they go, so a client sees a run start and finish without asking for the plan.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use forge_config::ci_parity::report::{CiReport, StepResult, StepStatus};

use crate::remote::ci::{SnapCi, SnapCiBlock, SnapCiRunning, SnapCiStep};

#[derive(Default)]
struct Registry {
    by_root: BTreeMap<PathBuf, SnapCi>,
    looked_up: BTreeSet<PathBuf>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    by_root: BTreeMap::new(),
    looked_up: BTreeSet::new(),
});
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Bumped on every change; a driver that only rebuilds its snapshot when something happened
/// compares it to know an idle session's run just started or finished.
pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// One consumer's view of [`generation`]: `changed()` is true once per change.
#[derive(Default)]
pub(crate) struct Watch(u64);

impl Watch {
    pub(crate) fn changed(&mut self) -> bool {
        let now = generation();
        std::mem::replace(&mut self.0, now) != now
    }
}

fn with_entry(root: &Path, f: impl FnOnce(&mut SnapCi)) {
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    f(reg.by_root.entry(root.to_path_buf()).or_default());
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

fn status_word(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Pass | StepStatus::Cached => "pass",
        StepStatus::Fail | StepStatus::Timeout => "fail",
        StepStatus::Skipped => "skip",
        StepStatus::NotRun => "not_run",
    }
}

fn step_of(result: &StepResult) -> SnapCiStep {
    SnapCiStep {
        key: result.key.clone(),
        name: result.name.clone(),
        status: status_word(result.status),
        secs: result.secs,
    }
}

pub(crate) fn begin(root: &Path, runnable: usize, skipped: usize) {
    with_entry(root, |ci| {
        ci.runnable = runnable;
        ci.skipped = skipped;
        ci.steps.clear();
        ci.running = None;
        ci.ok = None;
        ci.finished_unix = None;
    });
}

pub(crate) fn step_started(root: &Path, name: &str, index: usize, total: usize) {
    with_entry(root, |ci| {
        ci.running = Some(SnapCiRunning {
            name: name.to_string(),
            index,
            total,
        });
    });
}

pub(crate) fn step_done(root: &Path, result: &StepResult) {
    with_entry(root, |ci| {
        ci.steps.push(step_of(result));
        ci.running = None;
    });
}

pub(crate) fn finished(root: &Path, report: &CiReport) {
    with_entry(root, |ci| {
        ci.running = None;
        ci.finished_unix = Some(report.finished_unix);
        ci.ok = Some(report.ok());
        if report.ok() {
            ci.blocked = None;
        }
    });
}

/// A goal's finish gate or a push was refused on this run's first failing step.
pub(crate) fn blocked(root: &Path, via: &'static str, report: &CiReport) {
    let Some(failed) = report.first_failure() else {
        return;
    };
    with_entry(root, |ci| {
        ci.blocked = Some(SnapCiBlock {
            key: failed.key.clone(),
            name: failed.name.clone(),
            via,
        });
    });
}

fn from_report(report: &CiReport) -> SnapCi {
    SnapCi {
        runnable: report.results.len(),
        skipped: report.skipped.len(),
        running: None,
        steps: report.results.iter().map(step_of).collect(),
        finished_unix: Some(report.finished_unix),
        ok: Some(report.ok()),
        blocked: None,
    }
}

/// The summary for the repository a session works in: this process's run state, else the last
/// saved run (read once per directory). `None` until the repository has had a run.
pub(crate) fn for_cwd(cwd: &Path) -> Option<SnapCi> {
    if cwd.as_os_str().is_empty() {
        return None;
    }
    let mut reg = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = cwd.ancestors().find_map(|dir| reg.by_root.get(dir)) {
        return Some(found.clone());
    }
    if reg.looked_up.insert(cwd.to_path_buf()) {
        if let Some(report) = forge_config::ci_parity::report::load_report(cwd) {
            let ci = from_report(&report);
            reg.by_root.insert(cwd.to_path_buf(), ci.clone());
            return Some(ci);
        }
    }
    None
}

#[cfg(test)]
mod tests;
