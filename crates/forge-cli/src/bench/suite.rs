//! `forge bench tasks --suite <dir>`: Terminal-Bench-shaped task suites without container
//! orchestration in Forge.
//!
//! A suite is a directory of task directories. A task holds `instruction.md` (the prompt),
//! an optional `setup.sh` and a `test.sh` (exit 0 = solved). Each attempt gets a fresh temp
//! workspace: `setup.sh` runs in it, the agent works in it, then `test.sh` grades it. The scripts
//! stay in the task directory, outside the workspace, so the agent is not handed the grader.
//! Scripts run in their own session (`setsid`) so background jobs they start survive until the
//! attempt ends, and are then killed with the whole process group.
//!
//! Output mirrors `bench swe`: `<out>` (one result row per attempt), `<out>.metrics.jsonl` (the
//! same `InstanceMetric` sidecar, so `bench report` and `bench export` read it unchanged) and
//! `<out>.eval.json` (`resolved_ids`, the shape `bench report --run` pairs with metrics).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;

use super::{metrics_path, metrics_to_jsonl, outcome, seed_path, Agent, InstanceMetric};

pub struct TasksOptions {
    pub suite: PathBuf,
    pub out: PathBuf,
    pub limit: Option<usize>,
    pub model: Option<String>,
    pub agent: Agent,
    pub mock: bool,
    pub attempts: usize,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SuiteTask {
    pub name: String,
    pub dir: PathBuf,
    pub instruction: String,
    pub setup: Option<PathBuf>,
    pub test: PathBuf,
}

/// One row of `<out>`: how an attempt was graded.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(super) struct TaskResult {
    pub task: String,
    pub attempt: usize,
    pub solved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_exit: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_exit: Option<i32>,
    pub test_timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Every task directory under `suite` (those holding `instruction.md`), sorted by name.
pub(super) fn discover(suite: &Path) -> Result<Vec<SuiteTask>> {
    let entries =
        std::fs::read_dir(suite).with_context(|| format!("reading suite {}", suite.display()))?;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("instruction.md").is_file())
        .collect();
    dirs.sort();
    let mut tasks = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let dir = std::fs::canonicalize(&dir)
            .with_context(|| format!("resolving task {}", dir.display()))?;
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .context("task directory name is not UTF-8")?
            .to_string();
        let test = dir.join("test.sh");
        if !test.is_file() {
            anyhow::bail!("task {name} has instruction.md but no test.sh");
        }
        let instruction = std::fs::read_to_string(dir.join("instruction.md"))
            .with_context(|| format!("reading {name}/instruction.md"))?;
        let setup = Some(dir.join("setup.sh")).filter(|p| p.is_file());
        tasks.push(SuiteTask {
            name,
            dir,
            instruction,
            setup,
            test,
        });
    }
    if tasks.is_empty() {
        anyhow::bail!(
            "no tasks in {} (a task is a directory with instruction.md and test.sh)",
            suite.display()
        );
    }
    Ok(tasks)
}

/// `tasks.jsonl` → `tasks.eval.json`.
pub(super) fn eval_path(out: &Path) -> PathBuf {
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("tasks");
    out.with_file_name(format!("{stem}.eval.json"))
}

fn logs_dir(out: &Path) -> PathBuf {
    let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("tasks");
    out.with_file_name(format!("{stem}.logs"))
}

pub(super) fn eval_json(results: &[TaskResult]) -> serde_json::Value {
    let (solved, unsolved): (Vec<&TaskResult>, Vec<&TaskResult>) =
        results.iter().partition(|r| r.solved);
    serde_json::json!({
        "submitted_instances": results.len(),
        "resolved_instances": solved.len(),
        "resolved_ids": solved.iter().map(|r| &r.task).collect::<Vec<_>>(),
        "unresolved_ids": unsolved.iter().map(|r| &r.task).collect::<Vec<_>>(),
    })
}

fn results_to_jsonl(results: &[TaskResult]) -> String {
    results
        .iter()
        .map(|r| serde_json::to_string(r).expect("TaskResult serializes") + "\n")
        .collect()
}

struct ScriptRun {
    exit: Option<i32>,
    timed_out: bool,
}

/// Process groups started by one attempt's scripts; killed when the attempt ends so a server a
/// `setup.sh` left running cannot leak into the next attempt.
#[derive(Default)]
struct Groups(Vec<i32>);

impl Groups {
    #[cfg_attr(not(unix), allow(unused_variables))]
    fn forget_if_empty(&mut self, pg: i32) {
        #[cfg(unix)]
        if unsafe { libc::kill(-pg, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            self.0.retain(|&g| g != pg);
        }
    }

    fn kill_all(&mut self) {
        #[cfg(unix)]
        for pg in self.0.drain(..) {
            unsafe { libc::kill(-pg, libc::SIGKILL) };
        }
    }
}

impl Drop for Groups {
    fn drop(&mut self) {
        self.kill_all();
    }
}

async fn run_script(
    script: &Path,
    task: &SuiteTask,
    workspace: &Path,
    log: &Path,
    timeout: Duration,
    groups: &mut Groups,
) -> Result<ScriptRun> {
    let log_file =
        std::fs::File::create(log).with_context(|| format!("creating {}", log.display()))?;
    let mut command = tokio::process::Command::new("bash");
    command
        .arg(script)
        .current_dir(workspace)
        .env("TASK_DIR", &task.dir)
        .env("WORKSPACE", workspace)
        .stdin(std::process::Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);
    #[cfg(unix)]
    unsafe {
        // Safety: setsid() is async-signal-safe and valid between fork and exec (same pattern
        // as bridge_stats.rs). The child leads a new session and process group (pgid == pid).
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("starting {}", script.display()))?;
    let pgid = child.id().map(|pid| pid as i32);
    if let Some(pg) = pgid {
        groups.0.push(pg);
    }
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => {
            let exit = status?.code();
            // An empty group's id is free for the kernel to hand out again; forget it now so the
            // end-of-attempt kill cannot hit an unrelated process group that reused it.
            if let Some(pg) = pgid {
                groups.forget_if_empty(pg);
            }
            Ok(ScriptRun {
                exit,
                timed_out: false,
            })
        }
        Err(_) => {
            groups.kill_all();
            let _ = child.wait().await;
            Ok(ScriptRun {
                exit: None,
                timed_out: true,
            })
        }
    }
}

/// Commit the post-setup state so the agent's changes can be read back as a diff.
fn baseline_commit(workspace: &Path) -> Result<()> {
    if workspace.join(".git").exists() {
        return Ok(());
    }
    super::run_git(workspace, &["init", "-q"])?;
    super::run_git(workspace, &["add", "-A"])?;
    super::run_git(
        workspace,
        &[
            "-c",
            "user.email=bench@forge.invalid",
            "-c",
            "user.name=forge-bench",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "task baseline",
        ],
    )?;
    Ok(())
}

pub(crate) async fn run_tasks(opts: TasksOptions) -> Result<()> {
    if opts.mock && opts.agent != Agent::Forge {
        anyhow::bail!("--mock only applies to --agent forge");
    }
    // The scripts and the agent run with the task workspace as the process CWD, so a relative
    // `--out` (the default `tasks.jsonl`) would resolve its log paths inside the workspace.
    let opts = TasksOptions {
        out: std::path::absolute(&opts.out)
            .with_context(|| format!("resolving {}", opts.out.display()))?,
        ..opts
    };
    let mut tasks = discover(&opts.suite)?;
    if let Some(n) = opts.limit {
        tasks.truncate(n);
    }
    let attempts = opts.attempts.max(1);
    for seed in 1..=attempts {
        let out = if attempts == 1 {
            opts.out.clone()
        } else {
            eprintln!("=== attempt {seed}/{attempts} ===");
            seed_path(&opts.out, seed)
        };
        run_suite_once(&tasks, &opts, seed, &out).await?;
    }
    Ok(())
}

async fn run_suite_once(
    tasks: &[SuiteTask],
    opts: &TasksOptions,
    attempt: usize,
    out: &Path,
) -> Result<()> {
    let logs = logs_dir(out);
    std::fs::create_dir_all(&logs).with_context(|| format!("creating {}", logs.display()))?;
    let orig_cwd = std::env::current_dir().context("reading current dir")?;
    let mut results = Vec::with_capacity(tasks.len());
    let mut metrics = Vec::with_capacity(tasks.len());
    for (i, task) in tasks.iter().enumerate() {
        eprintln!("[{}/{}] {}", i + 1, tasks.len(), task.name);
        let (result, metric) = run_task(task, opts, attempt, &logs).await;
        let _ = std::env::set_current_dir(&orig_cwd);
        eprintln!(
            "  {} test.sh {}",
            if result.solved { "✓" } else { "✗" },
            match (result.test_exit, result.test_timed_out) {
                (_, true) => "timed out".to_string(),
                (Some(code), _) => format!("exited {code}"),
                (None, _) => "did not run".to_string(),
            }
        );
        results.push(result);
        metrics.push(metric);
    }
    std::fs::write(out, results_to_jsonl(&results))
        .with_context(|| format!("writing {}", out.display()))?;
    let metrics_out = metrics_path(out);
    std::fs::write(&metrics_out, metrics_to_jsonl(&metrics))
        .with_context(|| format!("writing {}", metrics_out.display()))?;
    let eval_out = eval_path(out);
    let eval = eval_json(&results);
    std::fs::write(
        &eval_out,
        serde_json::to_string_pretty(&eval).expect("eval serializes") + "\n",
    )
    .with_context(|| format!("writing {}", eval_out.display()))?;
    let solved = results.iter().filter(|r| r.solved).count();
    eprintln!(
        "solved {solved}/{} · results → {} · metrics → {} · eval → {} · logs → {}",
        results.len(),
        out.display(),
        metrics_out.display(),
        eval_out.display(),
        logs.display()
    );
    Ok(())
}

async fn run_task(
    task: &SuiteTask,
    opts: &TasksOptions,
    attempt: usize,
    logs: &Path,
) -> (TaskResult, InstanceMetric) {
    let mut result = TaskResult {
        task: task.name.clone(),
        attempt,
        solved: false,
        setup_exit: None,
        test_exit: None,
        test_timed_out: false,
        session_id: None,
    };
    let mut groups = Groups::default();
    let timeout = Duration::from_secs(opts.timeout_secs);
    let log = |kind: &str| logs.join(format!("{}.attempt{attempt}.{kind}.log", task.name));
    let started = std::time::Instant::now();
    let run = async {
        let tmp = tempfile::Builder::new()
            .prefix(&format!("forge-task-{}-", task.name))
            .tempdir()
            .context("creating task workspace")?;
        let workspace = std::fs::canonicalize(tmp.path())?;
        if let Some(setup) = &task.setup {
            let s =
                run_script(setup, task, &workspace, &log("setup"), timeout, &mut groups).await?;
            result.setup_exit = s.exit;
            if s.timed_out || s.exit != Some(0) {
                anyhow::bail!(
                    "setup.sh {}",
                    if s.timed_out {
                        "timed out".to_string()
                    } else {
                        format!("exited {:?}", s.exit)
                    }
                );
            }
        }
        baseline_commit(&workspace)?;
        std::env::set_current_dir(&workspace).context("entering task workspace")?;
        let mut outcome = super::run_agent(&super::AgentRun {
            agent: opts.agent,
            prompt: &task.instruction,
            dir: &workspace,
            model: opts.model.clone(),
            timeout_secs: opts.timeout_secs,
            mock: opts.mock,
            expect_code_change: false,
        })
        .await
        .unwrap_or_else(|e| super::RunOutcome::failed(format!("{e:#}")));
        outcome.patch = super::extract_patch(&workspace).unwrap_or_default();
        // A grader that cannot start is recorded on the attempt, but must not discard the agent
        // turn's session id and usage, which already happened.
        match run_script(
            &task.test,
            task,
            &workspace,
            &log("test"),
            timeout,
            &mut groups,
        )
        .await
        {
            Ok(t) => {
                result.test_exit = t.exit;
                result.test_timed_out = t.timed_out;
                result.solved = !t.timed_out && t.exit == Some(0);
            }
            Err(e) => {
                outcome
                    .error
                    .get_or_insert_with(|| format!("test.sh: {e:#}"));
            }
        }
        Ok::<_, anyhow::Error>(outcome)
    };
    let mut outcome = run
        .await
        .unwrap_or_else(|e| super::RunOutcome::failed(format!("{e:#}")));
    groups.kill_all();
    outcome.wall_secs = started.elapsed().as_secs_f64();
    if let Some(err) = &outcome.error {
        eprintln!("  ✗ {err}");
    }
    result.session_id = outcome.session_id.clone();
    let patched = !outcome.patch.is_empty();
    let (attempt_outcome, outcome_detail) = outcome::classify(&outcome::AttemptSignals {
        tools_unavailable: outcome.tools_unavailable,
        timed_out: outcome.timed_out,
        patched,
        final_text: outcome.final_text.as_deref(),
        error: outcome.error.as_deref(),
    });
    let metric = InstanceMetric {
        instance_id: task.name.clone(),
        agent: opts.agent.label().to_string(),
        input_tokens: outcome.input_tokens,
        output_tokens: outcome.output_tokens,
        total_tokens: outcome.input_tokens + outcome.output_tokens,
        cost_usd: outcome.cost_usd,
        wall_secs: outcome.wall_secs,
        patched,
        metrics_complete: outcome.metrics_complete,
        timed_out: outcome.timed_out,
        tools_unavailable: outcome.tools_unavailable,
        runtime: None,
        attempt,
        session_id: outcome.session_id,
        cached_input_tokens: outcome.cached_input_tokens,
        cache_write_tokens: outcome.cache_write_tokens,
        reasoning_tokens: outcome.reasoning_tokens,
        model_calls: outcome.model_calls,
        tool_calls: outcome.tool_calls,
        notional_cost_usd: outcome.notional_cost_usd,
        outcome: attempt_outcome,
        outcome_detail,
    };
    (result, metric)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn discover_lists_task_dirs_sorted_and_skips_non_tasks() {
        let suite = tempfile::tempdir().unwrap();
        let root = suite.path();
        write(&root.join("b-task/instruction.md"), "do b");
        write(&root.join("b-task/test.sh"), "exit 0");
        write(&root.join("a-task/instruction.md"), "do a");
        write(&root.join("a-task/setup.sh"), "true");
        write(&root.join("a-task/test.sh"), "exit 1");
        write(&root.join("notes/README.md"), "not a task");
        let tasks = discover(root).unwrap();
        let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a-task", "b-task"]);
        assert!(tasks[0].setup.is_some());
        assert!(tasks[1].setup.is_none());
        assert_eq!(tasks[0].instruction, "do a");
    }

    #[test]
    fn a_task_without_test_sh_is_an_error_not_a_silent_skip() {
        let suite = tempfile::tempdir().unwrap();
        write(&suite.path().join("t/instruction.md"), "x");
        let err = discover(suite.path()).unwrap_err().to_string();
        assert!(err.contains("no test.sh"), "{err}");
    }

    #[test]
    fn an_empty_suite_is_an_error() {
        let suite = tempfile::tempdir().unwrap();
        assert!(discover(suite.path()).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_background_job_from_setup_outlives_the_script_until_the_attempt_ends() {
        let suite = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let script = suite.path().join("t/setup.sh");
        write(&script, "sleep 600 &\necho $! > bg.pid\n");
        let task = SuiteTask {
            name: "t".into(),
            dir: suite.path().join("t"),
            instruction: String::new(),
            setup: Some(script.clone()),
            test: script.clone(),
        };
        let mut groups = Groups::default();
        let run = run_script(
            &script,
            &task,
            ws.path(),
            &suite.path().join("setup.log"),
            Duration::from_secs(30),
            &mut groups,
        )
        .await
        .unwrap();
        assert_eq!(run.exit, Some(0));
        let pid: i32 = std::fs::read_to_string(ws.path().join("bg.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
        assert!(
            alive(pid),
            "the job must survive the script that started it"
        );
        groups.kill_all();
        let mut gone = false;
        for _ in 0..50 {
            // Reap is done by init once the job dies; poll until it disappears.
            if !alive(pid) || is_zombie(pid) {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(gone, "ending the attempt kills the script's process group");
    }

    #[cfg(unix)]
    fn is_zombie(pid: i32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| {
                s.rsplit(')')
                    .next()
                    .unwrap_or("")
                    .trim_start()
                    .starts_with('Z')
            })
            .unwrap_or(false)
    }

    #[test]
    fn eval_json_lists_solved_tasks_in_report_shape() {
        let row = |task: &str, solved| TaskResult {
            task: task.into(),
            attempt: 1,
            solved,
            setup_exit: None,
            test_exit: Some(if solved { 0 } else { 1 }),
            test_timed_out: false,
            session_id: None,
        };
        let v = eval_json(&[row("a", true), row("b", false)]);
        assert_eq!(v["resolved_ids"], serde_json::json!(["a"]));
        assert_eq!(v["unresolved_ids"], serde_json::json!(["b"]));
        assert_eq!(v["submitted_instances"], 2);
        assert_eq!(
            eval_path(Path::new("/r/tasks.seed2.jsonl")),
            PathBuf::from("/r/tasks.seed2.eval.json")
        );
    }
}
