//! `forge queue` — the overnight autopilot. Queue big tasks during the day, drain them headless
//! (`forge queue run`, typically fired by a `forge schedule` timer overnight): each task runs a
//! full agent turn in its own isolated git worktree, budget-capped and optionally assay-gated,
//! and leaves a review-ready `autopilot/<slug>` branch. `forge queue report` is the morning
//! digest. No daemon — the drain is a plain foreground command, schedulable like any other.

use std::io::BufRead;

use anyhow::{Context, Result};

use crate::*;

pub(crate) fn queue_cmd(cmd: Option<QueueCmd>) -> Result<()> {
    match cmd {
        None | Some(QueueCmd::List) => list_queue_cmd(),
        Some(QueueCmd::Add {
            task,
            budget,
            mode,
            model,
        }) => add_queue_cmd(task.join(" "), budget, mode, model),
        Some(QueueCmd::Remove { id }) => remove_queue_cmd(&id),
        Some(QueueCmd::Run { gate, max, mock }) => run_queue_cmd(gate, max, mock),
        Some(QueueCmd::Report) => report_queue_cmd(),
    }
}

fn add_queue_cmd(
    task: String,
    budget: Option<f64>,
    mode: Option<String>,
    model: Option<String>,
) -> Result<()> {
    if task.trim().is_empty() {
        anyhow::bail!("queue task is empty — `forge queue add \"<task>\" [--budget USD]`");
    }
    if let Some(b) = budget {
        if b <= 0.0 || b.is_nan() {
            anyhow::bail!("--budget must be a positive USD amount");
        }
    }
    let cwd = canonical_cwd()?;
    let store = open_store()?;
    let id = forge_types::new_id();
    store
        .add_queue_task(
            &id,
            task.trim(),
            &cwd,
            mode.as_deref(),
            model.as_deref(),
            budget,
        )
        .context("saving queue task")?;
    println!("✓ queued {}  {}", &id[..8], task.trim());
    println!("  drain with `forge queue run`, or schedule it:");
    println!(
        "  forge schedule add \"queue drain\" --at 23:00   (then have it run `forge queue run`)"
    );
    Ok(())
}

fn list_queue_cmd() -> Result<()> {
    let store = open_store()?;
    let tasks = store.list_queue_tasks(None).context("listing queue")?;
    if tasks.is_empty() {
        println!("queue is empty — `forge queue add \"<task>\" [--budget USD]`");
        return Ok(());
    }
    println!("{:<8}  {:<11}  {:<9}  task", "id", "status", "budget");
    for t in &tasks {
        let budget = t
            .budget_usd
            .map(|b| format!("${b:.2}"))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<8}  {:<11}  {:<9}  {}",
            &t.id[..t.id.len().min(8)],
            t.status,
            budget,
            truncate(&t.task, 70),
        );
    }
    Ok(())
}

fn remove_queue_cmd(id_prefix: &str) -> Result<()> {
    let store = open_store()?;
    let id = resolve_queue_id(&store, id_prefix)?;
    if store
        .remove_queue_task(&id)
        .context("removing queue task")?
    {
        println!("✓ removed {}", &id[..8]);
    } else {
        anyhow::bail!(
            "{} is running — a mid-drain task can't be removed",
            &id[..8]
        );
    }
    Ok(())
}

fn resolve_queue_id(store: &Store, prefix: &str) -> Result<String> {
    let mut matches = store
        .matching_queue_task_ids(prefix)
        .context("looking up queue task")?;
    match matches.len() {
        0 => anyhow::bail!("no queue task matching '{prefix}' — see `forge queue list`"),
        1 => Ok(matches.remove(0)),
        n => anyhow::bail!("'{prefix}' is ambiguous ({n} tasks match) — use more characters"),
    }
}

// ---------------------------------------------------------------------------
// The drain. Sequential on purpose: overnight tasks are big, and one task's branch shouldn't
// race another's edits to the same files. Parallelism, when it comes, belongs at the mesh
// layer (per-provider caps), not here.
// ---------------------------------------------------------------------------

fn run_queue_cmd(gate: Option<String>, max: Option<usize>, mock: bool) -> Result<()> {
    let cwd = canonical_cwd()?;
    let repo_root = git_repo_root(&cwd)
        .context("`forge queue run` needs a git repository (results are left as branches)")?;
    let store = open_store()?;
    let pending: Vec<_> = store
        .list_queue_tasks(Some(&cwd))
        .context("listing queue")?
        .into_iter()
        .filter(|t| t.status == "pending")
        .take(max.unwrap_or(usize::MAX))
        .collect();
    if pending.is_empty() {
        println!("nothing pending for {cwd} — `forge queue add` first");
        return Ok(());
    }
    let forge_exe = std::env::current_exe().context("resolving the forge binary path")?;
    println!("draining {} task(s) from the queue…", pending.len());

    let mut finished: Vec<(String, String)> = Vec::new();
    for t in pending {
        if !store.claim_queue_task(&t.id, now())? {
            continue; // a concurrent drain got there first
        }
        println!("▶ {}  {}", &t.id[..8], truncate(&t.task, 70));
        let outcome = run_one_task(&t, &repo_root, &forge_exe, gate.as_deref(), mock);
        let (status, session_id, branch, summary, cost, gate_note) = match outcome {
            Ok(o) => (
                o.status,
                o.session_id,
                o.branch,
                o.summary,
                o.cost_usd,
                o.gate,
            ),
            Err(e) => (
                "failed".to_string(),
                None,
                None,
                Some(format!("{e:#}")),
                None,
                None,
            ),
        };
        store.finish_queue_task(
            &t.id,
            &status,
            now(),
            session_id.as_deref(),
            branch.as_deref(),
            summary.as_deref(),
            cost,
            gate_note.as_deref(),
        )?;
        let branch_note = branch.as_deref().unwrap_or("-");
        println!(
            "  {} {}  branch: {}",
            status_glyph(&status),
            status,
            branch_note
        );
        finished.push((status, t.task.clone()));
    }

    let done = finished.iter().filter(|(s, _)| s == "done").count();
    let line = format!(
        "forge queue: {}/{} task(s) produced a branch — `forge queue report` for the digest",
        done,
        finished.len()
    );
    println!("{line}");
    notify_desktop("Forge autopilot finished", &line);
    Ok(())
}

/// Everything recorded about one drained task.
struct TaskOutcome {
    status: String,
    session_id: Option<String>,
    branch: Option<String>,
    summary: Option<String>,
    cost_usd: Option<f64>,
    gate: Option<String>,
}

fn run_one_task(
    t: &forge_store::QueueTask,
    repo_root: &std::path::Path,
    forge_exe: &std::path::Path,
    gate: Option<&str>,
    mock: bool,
) -> Result<TaskOutcome> {
    let child_id = format!("queue-{}", &t.id[..t.id.len().min(12)]);
    let guard = forge_core::worktree::WorktreeGuard::create(repo_root, &child_id)
        .map_err(|e| anyhow::anyhow!("creating worktree: {e}"))?;

    // One headless agent turn inside the worktree. accept-edits unless the task pinned a mode:
    // the worktree IS the sandbox, and a prompt would hang forever with nobody watching.
    let mode = t.mode.as_deref().unwrap_or("accept-edits");
    let mut cmd = std::process::Command::new(forge_exe);
    cmd.arg("run")
        .arg(&t.task)
        .args(["--output-format", "stream-json", "--mode", mode])
        .current_dir(guard.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null());
    if let Some(m) = &t.model {
        cmd.args(["--model", m]);
    }
    if mock {
        cmd.arg("--mock");
    }
    let mut child = cmd.spawn().context("spawning the task run")?;
    let stdout = child.stdout.take().expect("piped stdout");

    let mut run = StreamRun::default();
    let mut over_budget = false;
    for line in std::io::BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        run.fold_line(&line);
        if let (Some(budget), Some(cost)) = (t.budget_usd, run.cost_usd) {
            if cost > budget {
                let _ = child.kill();
                over_budget = true;
                break;
            }
        }
    }
    let exit = child.wait().context("waiting for the task run")?;

    // Gate BEFORE committing: assay's diff scope reads the uncommitted working tree.
    let mut gate_note = None;
    if let Some(sev) = gate {
        if !over_budget {
            let gate_out = std::process::Command::new(forge_exe)
                .args(["assay", "run", "--scope", "diff", "--fail-on", sev])
                .current_dir(guard.path())
                .output()
                .context("running the assay gate")?;
            if gate_out.status.code() == Some(2) {
                gate_note = Some(format!("assay found ≥{sev} severity findings"));
            }
        }
    }

    let changed = forge_core::worktree::commit_worktree(guard.path())
        .map_err(|e| anyhow::anyhow!("committing worktree results: {e}"))?;

    // Keep the results on a branch that survives the guard's cleanup: the guard deletes its own
    // `forge/subagent/<id>` branch on drop, so a second ref must exist BEFORE drop.
    let branch = if changed {
        let name = format!("autopilot/{}-{}", slugify(&t.task), &t.id[..6]);
        let out = std::process::Command::new("git")
            .args(["branch", "-f", &name, guard.branch()])
            .current_dir(repo_root)
            .output()
            .context("creating the autopilot branch")?;
        if !out.status.success() {
            anyhow::bail!(
                "keeping the result branch failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Some(name)
    } else {
        None
    };

    let status = if over_budget {
        "over-budget"
    } else if gate_note.is_some() {
        "gated"
    } else if !exit.success() && branch.is_none() {
        "failed"
    } else if branch.is_some() {
        "done"
    } else {
        "empty"
    };
    Ok(TaskOutcome {
        status: status.to_string(),
        session_id: run.session_id,
        branch,
        summary: run.result.map(|s| truncate(&s, 400).to_string()),
        cost_usd: run.cost_usd,
        gate: gate_note,
    })
}

fn report_queue_cmd() -> Result<()> {
    let store = open_store()?;
    let mut tasks: Vec<_> = store
        .list_queue_tasks(None)
        .context("listing queue")?
        .into_iter()
        .filter(|t| t.finished_at.is_some())
        .collect();
    if tasks.is_empty() {
        println!("no drained tasks yet — `forge queue run` first");
        return Ok(());
    }
    tasks.sort_by_key(|t| std::cmp::Reverse(t.finished_at));
    tasks.truncate(20);
    println!("autopilot digest — most recent first\n");
    for t in &tasks {
        let cost = t
            .cost_usd
            .map(|c| format!("${c:.2}"))
            .unwrap_or_else(|| "-".into());
        println!(
            "{} {}  {}  [{}]  cost {}",
            status_glyph(&t.status),
            &t.id[..t.id.len().min(8)],
            truncate(&t.task, 60),
            t.status,
            cost
        );
        if let Some(b) = &t.branch {
            println!("    branch: {b}   (review: `git diff main...{b}`)");
        }
        if let Some(g) = &t.gate {
            println!("    gate:   {g}");
        }
        if let Some(s) = &t.summary {
            println!("    {}", truncate(s, 120));
        }
        if let Some(sid) = &t.session_id {
            println!("    replay: forge replay {}", &sid[..sid.len().min(8)]);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested below). The stream-json fold is the contract with `forge run
// --output-format stream-json`: init → session_id, usage → cumulative cost, result → final text.
// ---------------------------------------------------------------------------

/// Folded view of one child run's NDJSON stream.
#[derive(Default)]
struct StreamRun {
    session_id: Option<String>,
    cost_usd: Option<f64>,
    result: Option<String>,
}

impl StreamRun {
    fn fold_line(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        match (
            v.get("type").and_then(|t| t.as_str()),
            v.get("subtype").and_then(|t| t.as_str()),
        ) {
            (Some("system"), Some("init")) => {
                self.session_id = v
                    .get("session_id")
                    .and_then(|s| s.as_str())
                    .map(str::to_string);
            }
            (Some("system"), Some("usage")) => {
                self.cost_usd = v.get("total_cost_usd").and_then(|c| c.as_f64());
            }
            (Some("result"), _) => {
                self.result = v.get("result").and_then(|r| r.as_str()).map(str::to_string);
            }
            _ => {}
        }
    }
}

/// Lowercase, alnum-preserving, dash-separated slug for the result branch name (≤24 chars).
fn slugify(task: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true;
    for c in task.chars() {
        if slug.len() >= 24 {
            break;
        }
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "task".into()
    } else {
        slug
    }
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn status_glyph(status: &str) -> &'static str {
    match status {
        "done" => "✓",
        "empty" => "○",
        "gated" => "⚠",
        "over-budget" => "$",
        "failed" => "✗",
        "running" => "▶",
        _ => "·",
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn canonical_cwd() -> Result<String> {
    let cwd = std::env::current_dir().context("resolving the current directory")?;
    Ok(cwd
        .canonicalize()
        .unwrap_or(cwd)
        .to_string_lossy()
        .into_owned())
}

fn git_repo_root(cwd: &str) -> Result<std::path::PathBuf> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .context("running git")?;
    if !out.status.success() {
        anyhow::bail!("not a git repository");
    }
    Ok(std::path::PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim(),
    ))
}

/// Fire-and-forget desktop notification so an overnight drain announces itself in the morning.
/// Best-effort on every platform; failures (no DE, no notifier binary) are silently ignored.
fn notify_desktop(title: &str, body: &str) {
    if cfg!(target_os = "linux") {
        let _ = std::process::Command::new("notify-send")
            .args([title, body])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    } else if cfg!(target_os = "macos") {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "'"),
            title.replace('"', "'")
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    } else if cfg!(target_os = "windows") {
        // msg.exe is present on all supported Windows editions; a toast would need a helper
        // module or PowerShell body — this stays dependency-free and disappears on its own.
        let _ = std::process::Command::new("msg")
            .args(["*", "/TIME:30", &format!("{title}: {body}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_fold_extracts_session_cost_and_result() {
        let mut run = StreamRun::default();
        run.fold_line(r#"{"type":"system","subtype":"init","session_id":"abc123"}"#);
        run.fold_line(r#"{"type":"system","subtype":"usage","total_cost_usd":0.5,"usage":{}}"#);
        run.fold_line(r#"{"type":"system","subtype":"usage","total_cost_usd":1.75,"usage":{}}"#);
        run.fold_line("not json at all");
        run.fold_line(r#"{"type":"result","subtype":"success","result":"moved the module"}"#);
        assert_eq!(run.session_id.as_deref(), Some("abc123"));
        assert_eq!(run.cost_usd, Some(1.75));
        assert_eq!(run.result.as_deref(), Some("moved the module"));
    }

    #[test]
    fn slugify_makes_branch_safe_names() {
        assert_eq!(
            slugify("Migrate the auth module!"),
            "migrate-the-auth-module"
        );
        assert_eq!(slugify("///"), "task");
        assert_eq!(
            slugify("a very long task name that exceeds the slug budget entirely"),
            "a-very-long-task-name-th"
        );
        assert!(!slugify("ends with symbols ???").ends_with('-'));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo wörld", 5), "héllo");
        assert_eq!(truncate("short", 100), "short");
    }
}
