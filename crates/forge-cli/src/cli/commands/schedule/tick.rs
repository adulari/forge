//! One tick of a schedule: the process every OS timer (and the app's "Run now") launches.
//!
//! Wrapping `forge run` instead of exec'ing it directly is what makes a run observable: the tick
//! writes a `schedule_run` row before it starts, enforces the schedule's runtime cap, retries with
//! backoff, and records how it ended. A tick that dies with the machine leaves a `running` row
//! behind, which the next tick or daemon read turns into `interrupted`.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use forge_store::{Schedule, Store};

use super::{resolve_schedule_id, uninstall_timer};
use crate::cli::commands::replay::open_store;

pub(crate) const STOP_MARKER: &str = "[[schedule:done]]";
const MAX_RETRIES: i64 = 5;
const SUMMARY_CHARS: usize = 800;

pub(crate) fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Mark `running` rows whose tick process is gone as `interrupted`. Cheap enough to call on every
/// list and every tick.
pub(crate) fn sweep_dead_runs(store: &Store) -> Result<usize> {
    store
        .interrupt_dead_schedule_runs(
            |pid| forge_core::worktree_reclaim::pid_alive(pid as u32),
            unix_now(),
        )
        .context("sweeping interrupted schedule runs")
}

struct Outcome {
    status: &'static str,
    summary: Option<String>,
    error: Option<String>,
    stop_requested: bool,
}

/// The prompt a tick runs. A `stop_when` condition is a self-check the run answers itself: the
/// model is told to end with [`STOP_MARKER`] once the condition holds, and the tick pauses the
/// schedule when it sees it.
fn tick_prompt(schedule: &Schedule) -> String {
    match schedule.policy.stop_when.as_deref().map(str::trim) {
        Some(condition) if !condition.is_empty() => format!(
            "{}\n\nAfter doing the work above, decide whether this is now true: {condition}\n\
             If it is, end your final reply with the exact line {STOP_MARKER} and nothing after \
             it. If it is not, do not write that line.",
            schedule.task
        ),
        _ => schedule.task.clone(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}…")
}

fn drain<R: Read + Send + 'static>(mut reader: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

fn run_once(schedule: &Schedule, session_id: &str) -> Outcome {
    let exe = std::env::current_exe().unwrap_or_else(|_| "forge".into());
    let mut command = std::process::Command::new(exe);
    command
        .arg("run")
        .arg(tick_prompt(schedule))
        .args(["--output-format", "json", "--session-id", session_id])
        .current_dir(&schedule.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(mode) = schedule.mode.as_deref() {
        command.args(["--mode", mode]);
    }
    if let Some(model) = schedule.model.as_deref() {
        command.args(["--model", model]);
    }
    // Lets tests and demos drive a real tick without a provider key.
    if std::env::var_os("FORGE_SCHEDULE_TICK_MOCK").is_some() {
        command.arg("--mock");
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Outcome {
                status: "failed",
                summary: None,
                error: Some(format!("could not start forge run: {error}")),
                stop_requested: false,
            }
        }
    };
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let deadline = schedule
        .policy
        .max_runtime_secs
        .filter(|secs| *secs > 0)
        .map(|secs| Instant::now() + Duration::from_secs(secs as u64));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let out = stdout.and_then(|h| h.join().ok()).unwrap_or_default();
    let err = stderr.and_then(|h| h.join().ok()).unwrap_or_default();

    if timed_out {
        return Outcome {
            status: "failed",
            summary: None,
            error: Some(format!(
                "stopped after hitting the {}s runtime limit",
                schedule.policy.max_runtime_secs.unwrap_or_default()
            )),
            stop_requested: false,
        };
    }
    let parsed: Option<serde_json::Value> = serde_json::from_str(out.trim()).ok();
    let result_text = parsed
        .as_ref()
        .and_then(|v| v.get("result"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reported_error = parsed
        .as_ref()
        .and_then(|v| v.get("is_error"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let exited_ok = status.is_some_and(|s| s.success());
    let stop_requested = exited_ok && result_text.contains(STOP_MARKER);
    let summary_source = result_text.replace(STOP_MARKER, "");
    let summary =
        (!summary_source.trim().is_empty()).then(|| truncate(&summary_source, SUMMARY_CHARS));
    if exited_ok && !reported_error {
        Outcome {
            status: "ok",
            summary,
            error: None,
            stop_requested,
        }
    } else {
        let detail = if !err.trim().is_empty() {
            truncate(err.trim().lines().last().unwrap_or(""), 400)
        } else if let Some(text) = &summary {
            text.clone()
        } else {
            match status.and_then(|s| s.code()) {
                Some(code) => format!("forge run exited with code {code}"),
                None => "forge run ended without a result".to_string(),
            }
        };
        Outcome {
            status: "failed",
            summary,
            error: Some(detail),
            stop_requested: false,
        }
    }
}

fn pause(store: &Store, id: &str) -> Result<()> {
    uninstall_timer(id)?;
    store.set_schedule_enabled(id, false)?;
    Ok(())
}

pub(crate) fn tick_cmd(id_prefix: &str, manual: bool) -> Result<()> {
    let store = open_store()?;
    let id = resolve_schedule_id(&store, id_prefix)?;
    let schedule = store
        .get_schedule(&id)?
        .ok_or_else(|| anyhow::anyhow!("no such schedule"))?;
    if !manual && !schedule.enabled {
        return Ok(());
    }
    sweep_dead_runs(&store)?;
    let trigger = if manual { "manual" } else { "timer" };
    let pid = i64::from(std::process::id());

    if !store.running_schedule_runs(&id)?.is_empty() {
        let run_id = forge_types::new_id();
        let now = unix_now();
        store.begin_schedule_run(&run_id, &id, trigger, 1, None, Some(pid), now)?;
        store.finish_schedule_run(
            &run_id,
            "skipped",
            None,
            Some("the previous run was still running"),
            now,
        )?;
        return Ok(());
    }

    let attempts = 1 + schedule.policy.retries.clamp(0, MAX_RETRIES);
    let mut last = None;
    for attempt in 1..=attempts {
        let run_id = forge_types::new_id();
        let session_id = forge_types::new_id();
        store.begin_schedule_run(
            &run_id,
            &id,
            trigger,
            attempt,
            Some(&session_id),
            Some(pid),
            unix_now(),
        )?;
        let outcome = run_once(&schedule, &session_id);
        store.finish_schedule_run(
            &run_id,
            outcome.status,
            outcome.summary.as_deref(),
            outcome.error.as_deref(),
            unix_now(),
        )?;
        let ok = outcome.status == "ok";
        last = Some(outcome);
        if ok || attempt == attempts {
            break;
        }
        let wait = schedule
            .policy
            .backoff_secs
            .max(0)
            .saturating_mul(1_i64 << (attempt - 1).min(10))
            .min(3600);
        std::thread::sleep(Duration::from_secs(wait as u64));
    }

    let Some(outcome) = last else {
        return Ok(());
    };
    let reached_cap = schedule
        .policy
        .max_iterations
        .is_some_and(|cap| store.count_finished_schedule_runs(&id).unwrap_or(0) >= cap);
    let stop_on_error = outcome.status == "failed" && schedule.policy.stop_on_error;
    if outcome.stop_requested || reached_cap || stop_on_error {
        pause(&store, &id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(stop_when: Option<&str>) -> Schedule {
        Schedule {
            id: "s".into(),
            task: "check the build".into(),
            cwd: "/tmp".into(),
            mode: None,
            model: None,
            cron: "every:60".into(),
            enabled: true,
            created_at: 0,
            last_run: None,
            policy: forge_store::SchedulePolicy {
                stop_when: stop_when.map(str::to_string),
                ..Default::default()
            },
        }
    }

    #[test]
    fn prompt_is_untouched_without_a_stop_condition() {
        assert_eq!(tick_prompt(&schedule(None)), "check the build");
        assert_eq!(tick_prompt(&schedule(Some("  "))), "check the build");
    }

    #[test]
    fn stop_condition_asks_for_the_marker() {
        let prompt = tick_prompt(&schedule(Some("CI is green")));
        assert!(prompt.starts_with("check the build"));
        assert!(prompt.contains("CI is green"));
        assert!(prompt.contains(STOP_MARKER));
    }

    #[test]
    fn summaries_are_clipped_on_char_boundaries() {
        assert_eq!(truncate("  hi  ", 10), "hi");
        assert_eq!(truncate("ééééé", 3), "ééé…");
    }
}
