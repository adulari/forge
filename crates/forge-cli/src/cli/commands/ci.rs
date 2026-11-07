//! `forge ci plan|run|status`: the repository's CI, run locally (docs/features/ci-parity.md).

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use forge_config::ci_parity::plan::CiPlan;
use forge_config::ci_parity::report::{load_report, CiReport, StepStatus};

use crate::ci_run::{failure_text, make_plan, run_plan, skip_line, Event, RunOptions};
use crate::cli::args::ci::CiCmd;

fn root_of(project: Option<String>) -> Result<PathBuf> {
    let dir = match project {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    Ok(git_toplevel(&dir).unwrap_or(dir))
}

fn git_toplevel(dir: &std::path::Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

pub(crate) fn render_plan(plan: &CiPlan) -> String {
    let mut out = format!("{} step(s) would run locally\n", plan.steps.len());
    let mut i = 0;
    while i < plan.steps.len() {
        let (workflow, job) = (&plan.steps[i].workflow, &plan.steps[i].job);
        let group: Vec<_> = plan.steps[i..]
            .iter()
            .take_while(|s| s.workflow == *workflow && s.job == *job)
            .collect();
        i += group.len();
        // Environment shared by every step of the job is printed once under the job.
        let common: Vec<(&String, &String)> = group[0]
            .env
            .iter()
            .filter(|(k, v)| group.iter().all(|s| s.env.get(*k) == Some(*v)))
            .collect();
        out.push_str(&format!("\n{workflow} / {job}\n"));
        if !common.is_empty() {
            let env: Vec<String> = common.iter().map(|(k, v)| format!("{k}={v}")).collect();
            out.push_str(&format!("  env: {}\n", env.join(" ")));
        }
        for step in group {
            out.push_str(&format!("  - {}\n", step.name));
            let first = step
                .run
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("");
            let more = if step.run.trim().lines().count() > 1 {
                "  [...]"
            } else {
                ""
            };
            out.push_str(&format!("      $ {first}{more}\n"));
            let extra: Vec<String> = step
                .env
                .iter()
                .filter(|(k, _)| !common.iter().any(|(c, _)| c == k))
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            if !extra.is_empty() {
                out.push_str(&format!("      env: {}\n", extra.join(" ")));
            }
            if let Some(cwd) = &step.cwd {
                out.push_str(&format!("      in: {cwd}\n"));
            }
            if let Some(t) = step.timeout_secs {
                out.push_str(&format!("      timeout: {} min\n", t / 60));
            }
        }
    }
    out.push_str(&format!(
        "\n{} skipped ({} setup action(s) treated as no-ops)\n",
        plan.skips.len(),
        plan.noops
    ));
    for skip in &plan.skips {
        out.push_str(&format!("  - {}\n", skip_line(skip)));
    }
    out
}

fn render_report(report: &CiReport) -> String {
    let mut out = String::new();
    for r in &report.results {
        let mark = match r.status {
            StepStatus::Pass => "pass",
            StepStatus::Cached => "pass (unchanged tree)",
            StepStatus::Fail => "FAIL",
            StepStatus::Timeout => "TIMEOUT",
            StepStatus::NotRun => "not run",
            StepStatus::Skipped => "skipped (not installed)",
        };
        out.push_str(&format!("  {mark:<22} {} ({}s)\n", r.name, r.secs));
    }
    let (pass, fail, rest) = report.counts();
    out.push_str(&format!("{pass} passed, {fail} failed, {rest} not run"));
    if let Some(why) = &report.stopped {
        out.push_str(&format!(" - stopped: {why}"));
    }
    out.push('\n');
    out
}

pub(crate) fn ci_cmd(cmd: Option<CiCmd>) -> Result<()> {
    match cmd.unwrap_or(CiCmd::Plan {
        project: None,
        json: false,
    }) {
        CiCmd::Plan { project, json } => {
            let root = root_of(project)?;
            let (plan, settings) = make_plan(&root);
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
                return Ok(());
            }
            if let Some(w) = &settings.warning {
                eprintln!("warning: {w}");
            }
            print!("{}", render_plan(&plan));
            Ok(())
        }
        CiCmd::Run {
            project,
            keep_going,
            fresh,
            step,
            json,
        } => {
            let root = root_of(project)?;
            let (plan, settings) = make_plan(&root);
            if plan.steps.is_empty() {
                bail!("no runnable CI steps; see `forge ci plan` for what was skipped");
            }
            let opts = RunOptions {
                keep_going,
                fresh,
                only: step,
                cancel: None,
            };
            let report = run_plan(&root, &plan, &settings, &opts, &mut |event| {
                if json {
                    return;
                }
                match event {
                    Event::Start { index, total, step } => {
                        eprintln!("[{index}/{total}] {} ...", step.name);
                    }
                    Event::Done { result } => {
                        eprintln!("      {:?} ({}s)", result.status, result.secs)
                    }
                }
            })
            .map_err(|busy| anyhow::anyhow!(busy))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render_report(&report));
            }
            if report.ok() {
                return Ok(());
            }
            if !json {
                eprintln!("\n{}", failure_text(&plan, &report));
            }
            bail!("local CI failed")
        }
        CiCmd::Status { project, json } => {
            let root = root_of(project)?;
            let Some(report) = load_report(&root) else {
                bail!("no local CI run recorded for this project yet; run `forge ci run`");
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", render_report(&report));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_text_lists_env_and_reasoned_skips() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(
            wf.join("ci.yml"),
            "on: push\nenv:\n  RUSTFLAGS: -D warnings\njobs:\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - name: Clippy\n        run: cargo clippy\n      - uses: some/deploy@v1\n  win:\n    runs-on: windows-latest\n    steps:\n      - run: dir\n",
        )
        .unwrap();
        let (plan, _) = make_plan(dir.path());
        let text = render_plan(&plan);
        assert!(text.contains("ci.yml / lint"), "{text}");
        assert!(
            text.contains("$ cargo clippy") && text.contains("RUSTFLAGS=-D warnings"),
            "{text}"
        );
        assert!(
            text.contains("uses: some/deploy") && text.contains("windows-latest"),
            "{text}"
        );
    }
}
