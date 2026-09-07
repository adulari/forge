//! Background jobs: processes that are meant to outlive the tool call that started them.
//!
//! The foreground path deliberately SIGKILLs the whole process group once a command returns, so a
//! leaked descendant cannot hold the output pipes open and stall the turn. That is right for
//! `cmd &` inside a normal call — and fatal for the one case where the process IS the point: an
//! emulator, a dev server, a tunnel, a daemon under test. `nohup` does not save it, because nohup
//! only detaches from the terminal; the child keeps the shell's process group and the group kill
//! still reaches it. That trap cost a real session hours of "the sandbox kills my background
//! processes" before the mechanism was found.
//!
//! So a background job is spawned into its **own session** (`setsid`), which is what actually puts
//! it out of reach of any process-group kill, with stdout and stderr on a **log file** rather than
//! a pipe, so there is no descriptor for anything to hold open. The job then survives not just the
//! call but the Forge process itself.
//!
//! State lives on disk under `<workspace>/.forge/jobs/` — one `<pid>.json` record and one
//! `<pid>.log` per job — so a later call, a later turn, or a whole new session can still find,
//! read, and stop what an earlier one started. The job id IS the pid: there is no second
//! identifier to keep straight, and `kill <id>` from a human's terminal means the same thing.

use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use forge_types::SideEffect;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::sandbox::SandboxPolicy;
use crate::{str_arg, Tool, ToolError};

/// Bytes of log tail handed back by default. Small enough to keep a status check cheap; the log
/// file itself is always there for a full read.
pub const DEFAULT_TAIL_LINES: usize = 40;
/// Grace period between SIGTERM and SIGKILL when stopping a job.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Where job records and logs live for a workspace.
///
/// Project-local rather than a global state dir: the log of a dev server belongs next to the
/// project it serves, and a human debugging the same problem finds it without knowing Forge's
/// platform directories.
pub fn jobs_dir(workspace: Option<&Path>, cwd: &str) -> PathBuf {
    let root = workspace
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(cwd));
    root.join(".forge").join("jobs")
}

/// A job as it exists on disk.
pub struct Job {
    pub pid: u32,
    pub command: String,
    pub cwd: String,
    pub log: PathBuf,
    pub started_at: u64,
    /// `/proc/<pid>/stat` start time, when the platform has one. Guards against a recycled pid
    /// making a long-dead job look alive.
    start_ticks: Option<u64>,
    /// Exit code, once observed. `None` while running, or if Forge was not alive to see it.
    exit: Option<i64>,
}

impl Job {
    /// Whether the process is still there — the recorded exit wins, then pid liveness, then the
    /// start-time guard that rules out a recycled pid.
    pub fn is_running(&self) -> bool {
        if self.exit.is_some() {
            return false;
        }
        if !pid_alive(self.pid) {
            return false;
        }
        match (self.start_ticks, proc_start_ticks(self.pid)) {
            (Some(recorded), Some(current)) => recorded == current,
            _ => true,
        }
    }

    pub fn status(&self) -> String {
        match (self.is_running(), self.exit) {
            (true, _) => "running".to_string(),
            (false, Some(code)) => format!("exited {code}"),
            (false, None) => "gone".to_string(),
        }
    }

    /// One line for a listing.
    pub fn summary(&self) -> String {
        let age = now_secs().saturating_sub(self.started_at);
        format!(
            "{:<7} {:<10} {:>6}s  {}",
            self.pid,
            self.status(),
            age,
            first_line(&self.command)
        )
    }
}

/// Start a command as a background job and return its record.
///
/// The sandbox policy is applied exactly as it is for a foreground command: a background job is
/// not a way around confinement.
pub async fn start(
    command: &str,
    cwd: &str,
    workspace: Option<&Path>,
    policy: &SandboxPolicy,
) -> Result<Job, String> {
    if policy.enabled && !crate::sandbox::is_supported() {
        return Err(
            "refusing to start a background job unconfined: `shell.sandbox = true` is set but \
             this host cannot enforce it (no Landlock). Set `shell.sandbox = false` to accept \
             unconfined execution deliberately."
                .to_string(),
        );
    }
    let dir = jobs_dir(workspace, cwd);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    // The log has to exist before the spawn (it is the child's stdout), but the id is the pid,
    // which only exists after it. Open under a provisional name and rename once the pid is known —
    // the child's descriptor follows the file, not the path.
    let started_at = now_secs();
    let provisional = dir.join(format!(".starting-{started_at}-{}.log", std::process::id()));
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&provisional)
        .map_err(|e| format!("open log {}: {e}", provisional.display()))?;
    let log_for_err = log_file
        .try_clone()
        .map_err(|e| format!("clone log handle: {e}"))?;

    let (shell, flag) = super::shell_invocation();
    let mut cmd = Command::new(shell);
    cmd.arg(flag)
        .arg(command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(log_for_err))
        // Explicitly NOT kill_on_drop: outliving this call is the entire point.
        .kill_on_drop(false);
    detach_into_new_session(&mut cmd);
    super::maybe_install_sandbox(&mut cmd, policy, cwd, None);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_file(&provisional);
            return Err(format!("failed to start (cwd {cwd}): {error}"));
        }
    };
    let Some(pid) = child.id() else {
        return Err("the job exited before its pid could be read".to_string());
    };

    let log = dir.join(format!("{pid}.log"));
    let _ = std::fs::rename(&provisional, &log);
    let job = Job {
        pid,
        command: command.to_string(),
        cwd: cwd.to_string(),
        log,
        started_at,
        start_ticks: proc_start_ticks(pid),
        exit: None,
    };
    write_record(&dir, &job)?;

    // Watch for the exit so `status` can report a real code rather than only "gone". This task
    // also reaps the child, which is what keeps a finished job from lingering as a zombie and
    // reading as still-running. If Forge exits first the job is simply reparented to init and the
    // pid-liveness check takes over.
    let dir_for_task = dir.clone();
    tokio::spawn(async move {
        let code = match child.wait().await {
            Ok(status) => status.code().map(i64::from).unwrap_or(-1),
            Err(_) => return,
        };
        record_exit(&dir_for_task, pid, code);
    });

    Ok(job)
}

/// Every job recorded for this workspace, newest first.
pub fn list(workspace: Option<&Path>, cwd: &str) -> Vec<Job> {
    let dir = jobs_dir(workspace, cwd);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut jobs: Vec<Job> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| read_record(&entry.path()))
        .collect();
    jobs.sort_by_key(|job| std::cmp::Reverse(job.started_at));
    jobs
}

/// One job by id (its pid).
pub fn get(workspace: Option<&Path>, cwd: &str, id: u32) -> Option<Job> {
    read_record(&jobs_dir(workspace, cwd).join(format!("{id}.json")))
}

/// The last `lines` lines of a job's log.
///
/// Reads from the end of the file rather than the start: a job that has been running for an hour
/// has a log nobody wants to load whole, and the interesting part is always the tail.
pub fn tail(log: &Path, lines: usize) -> Result<String, String> {
    let mut file = std::fs::File::open(log).map_err(|e| format!("open {}: {e}", log.display()))?;
    let len = file
        .metadata()
        .map_err(|e| format!("stat {}: {e}", log.display()))?
        .len();
    // A generous per-line budget, capped: enough to hold the requested lines for normal output
    // without reading a multi-gigabyte log into memory.
    let window = (lines as u64 * 400).min(256 * 1024).min(len);
    file.seek(SeekFrom::Start(len - window))
        .map_err(|e| format!("seek {}: {e}", log.display()))?;
    let mut buffer = Vec::with_capacity(window as usize);
    std::io::Read::read_to_end(&mut file, &mut buffer)
        .map_err(|e| format!("read {}: {e}", log.display()))?;
    let text = String::from_utf8_lossy(&buffer);
    // A partial first line is an artefact of the window, not content — drop it unless we read the
    // whole file.
    let text = if window < len {
        text.split_once('\n').map(|(_, rest)| rest).unwrap_or(&text)
    } else {
        &text
    };
    let collected: Vec<&str> = text.lines().collect();
    let start = collected.len().saturating_sub(lines);
    Ok(collected[start..].join("\n"))
}

/// Stop a job: SIGTERM to its process group, then SIGKILL if it is still there.
///
/// The group, not the pid: a background job is a session leader, so its own children (the real
/// server behind a wrapper script) share its group and would otherwise be orphaned still-running.
pub async fn stop(workspace: Option<&Path>, cwd: &str, id: u32) -> Result<String, String> {
    let Some(job) = get(workspace, cwd, id) else {
        return Err(format!("no background job {id} in this workspace"));
    };
    if !job.is_running() {
        return Ok(format!("job {id} was already {}", job.status()));
    }
    signal_group(id, libc_sigterm());
    let deadline = std::time::Instant::now() + STOP_GRACE;
    while std::time::Instant::now() < deadline {
        if !pid_alive(id) {
            record_exit(&jobs_dir(workspace, cwd), id, -1);
            return Ok(format!("job {id} stopped"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    signal_group(id, libc_sigkill());
    record_exit(&jobs_dir(workspace, cwd), id, -1);
    Ok(format!(
        "job {id} killed (did not stop within {STOP_GRACE:?})"
    ))
}

/// Processes still alive in `pgid` that are not the (already exited) shell itself.
///
/// This is what makes the foreground group kill honest. Without it, `cmd &` in a normal call looks
/// like it worked — the call returns 0, the model moves on — and the process is SIGKILLed a
/// millisecond later with nothing said. Naming the casualties turns a silent, repeatable mystery
/// into one line the model can act on.
#[cfg(target_os = "linux")]
pub fn survivors_in_group(pgid: i32) -> Vec<(i32, String)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        if pid == pgid {
            continue; // the shell we just waited on
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `comm` is parenthesised and may contain spaces, so the fixed fields start after the
        // last ')': state, ppid, pgrp.
        let Some((name, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let (Some(state), Some(pgrp)) = (fields.first(), fields.get(2)) else {
            continue;
        };
        if *state == "Z" {
            continue; // already dead, just not reaped
        }
        if pgrp.parse::<i32>() == Ok(pgid) {
            let comm = name.rsplit_once('(').map(|(_, c)| c).unwrap_or("?");
            found.push((pid, comm.to_string()));
        }
    }
    found.sort();
    found
}

#[cfg(not(target_os = "linux"))]
pub fn survivors_in_group(_pgid: i32) -> Vec<(i32, String)> {
    Vec::new()
}

/// The note appended to a foreground result when the group kill actually killed something.
pub fn survivor_note(survivors: &[(i32, String)]) -> String {
    let listed: Vec<String> = survivors
        .iter()
        .take(4)
        .map(|(pid, comm)| format!("{comm}({pid})"))
        .collect();
    let more = survivors.len().saturating_sub(listed.len());
    let more = if more > 0 {
        format!(" +{more} more")
    } else {
        String::new()
    };
    format!(
        "  (killed {} process(es) this command left running: {}{more} — a foreground call does \
         not keep processes alive past its return, and `nohup`/`&` do not change that because the \
         child keeps this call's process group. To start something that must keep running, set \
         background:true)",
        survivors.len(),
        listed.join(", ")
    )
}

fn write_record(dir: &Path, job: &Job) -> Result<(), String> {
    let record = json!({
        "pid": job.pid,
        "command": job.command,
        "cwd": job.cwd,
        "log": job.log.display().to_string(),
        "started_at": job.started_at,
        "start_ticks": job.start_ticks,
        "exit": job.exit,
    });
    let path = dir.join(format!("{}.json", job.pid));
    std::fs::write(&path, format!("{record:#}\n"))
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn read_record(path: &Path) -> Option<Job> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some(Job {
        pid: value.get("pid")?.as_u64()? as u32,
        command: value.get("command")?.as_str()?.to_string(),
        cwd: value
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        log: PathBuf::from(value.get("log")?.as_str()?),
        started_at: value.get("started_at").and_then(Value::as_u64).unwrap_or(0),
        start_ticks: value.get("start_ticks").and_then(Value::as_u64),
        exit: value.get("exit").and_then(Value::as_i64),
    })
}

fn record_exit(dir: &Path, pid: u32, code: i64) {
    let path = dir.join(format!("{pid}.json"));
    let Some(mut job) = read_record(&path) else {
        return;
    };
    if job.exit.is_some() {
        return;
    }
    job.exit = Some(code);
    let _ = write_record(dir, &job);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn first_line(command: &str) -> String {
    let line = command.lines().next().unwrap_or_default();
    if line.chars().count() > 70 {
        let cut: String = line.chars().take(69).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

/// Put the child in a new session, which is what actually survives a process-group kill.
#[cfg(unix)]
fn detach_into_new_session(cmd: &mut Command) {
    // SAFETY: setsid is async-signal-safe and touches only the calling (freshly forked) process.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_into_new_session(_cmd: &mut Command) {}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) {
    unsafe { libc::kill(-(pid as i32), signal) };
}

#[cfg(not(unix))]
fn signal_group(_pid: u32, _signal: i32) {}

#[cfg(unix)]
fn libc_sigterm() -> i32 {
    libc::SIGTERM
}
#[cfg(not(unix))]
fn libc_sigterm() -> i32 {
    0
}
#[cfg(unix)]
fn libc_sigkill() -> i32 {
    libc::SIGKILL
}
#[cfg(not(unix))]
fn libc_sigkill() -> i32 {
    0
}

/// Process start time in clock ticks since boot (`/proc/<pid>/stat` field 22). Two processes with
/// the same pid but different start times are different processes.
#[cfg(target_os = "linux")]
fn proc_start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    // Field 22 overall; after `comm` the first entry is field 3, so index 19.
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
fn proc_start_ticks(_pid: u32) -> Option<u64> {
    None
}

// ---------------------------------------------------------------------------
// The tool surface: `shell{background:true}` starts a job, `shell_job` manages it.
// ---------------------------------------------------------------------------

/// The workspace bound to this session, when a call is running inside one.
pub(super) fn bound_workspace() -> Option<PathBuf> {
    crate::SESSION_WORKSPACE.try_with(Clone::clone).ok()
}

/// Manage the jobs `shell` started with `background: true`.
///
/// Separate from `shell` because its actions take a job id, not a command — and because the whole
/// value of a background job is that a LATER call can find it. A model that started an emulator in
/// one turn needs one obvious place to ask "is it still up, and what did it say".
#[derive(Default)]
pub struct ShellJobTool {
    workspace: Option<std::path::PathBuf>,
}

impl ShellJobTool {
    pub fn in_workspace(workspace: &std::path::Path) -> Self {
        Self {
            workspace: Some(workspace.to_path_buf()),
        }
    }

    /// The workspace whose jobs this call is about.
    ///
    /// The session binding wins over the field: the registry rebinds a live session to a new
    /// workspace (a worktree, say) without rebuilding tools, and a job list that ignored that
    /// would show the wrong project's jobs.
    fn workspace(&self) -> Option<std::path::PathBuf> {
        bound_workspace().or_else(|| self.workspace.clone())
    }
}

#[async_trait]
impl Tool for ShellJobTool {
    fn name(&self) -> &str {
        "shell_job"
    }
    fn description(&self) -> &str {
        "Inspect and control the long-lived jobs started by `shell` with background:true \
         (emulator, dev server, tunnel, daemon). Actions: list (every job and whether it is still \
         running), log (the tail of a job's output), status (one job), stop (SIGTERM then SIGKILL \
         the job and its children). Jobs survive across tool calls, turns, and Forge restarts, so \
         a job started earlier is still reachable here by its id."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Shell
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "log", "status", "stop"], "description": "list, log, status, or stop." },
                "id": { "type": "integer", "description": "Job id (its pid), from the shell background:true result or from list. Required for log/status/stop." },
                "lines": { "type": "integer", "minimum": 1, "description": "For log: how many trailing lines to return (default 40)." }
            },
            "required": ["action"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let action = str_arg(args, "action")?;
        let workspace = self.workspace();
        let cwd = workspace
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .display()
            .to_string();
        let workspace = workspace.as_deref();
        let id = || -> Result<u32, ToolError> {
            args.get("id")
                .and_then(Value::as_u64)
                .map(|id| id as u32)
                .ok_or_else(|| ToolError::Failed("this action needs a job id".into()))
        };

        match action {
            "list" => {
                let jobs = list(workspace, &cwd);
                if jobs.is_empty() {
                    return Ok(
                        "no background jobs in this workspace. Start one with shell \
                               background:true."
                            .to_string(),
                    );
                }
                let mut out = format!("{} background job(s):\n", jobs.len());
                out.push_str("pid     status       age  command\n");
                for job in &jobs {
                    out.push_str(&job.summary());
                    out.push('\n');
                }
                Ok(out)
            }
            "status" => {
                let id = id()?;
                let Some(job) = get(workspace, &cwd, id) else {
                    return Ok(format!("no background job {id} in this workspace"));
                };
                Ok(format!(
                    "job {} is {}\n  command: {}\n  cwd: {}\n  log: {}",
                    job.pid,
                    job.status(),
                    job.command,
                    job.cwd,
                    job.log.display()
                ))
            }
            "log" => {
                let id = id()?;
                let lines = args
                    .get("lines")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_TAIL_LINES as u64)
                    .clamp(1, 2000) as usize;
                let Some(job) = get(workspace, &cwd, id) else {
                    return Ok(format!("no background job {id} in this workspace"));
                };
                let body = tail(&job.log, lines).map_err(ToolError::Failed)?;
                let body = if body.trim().is_empty() {
                    "(no output yet)".to_string()
                } else {
                    body
                };
                Ok(format!(
                    "job {} is {} — last {lines} lines of {}:\n\n{body}",
                    job.pid,
                    job.status(),
                    job.log.display()
                ))
            }
            "stop" => {
                let id = id()?;
                stop(workspace, &cwd, id).await.map_err(ToolError::Failed)
            }
            other => Err(ToolError::Failed(format!(
                "unknown action '{other}'. Use list, log, status, or stop."
            ))),
        }
    }
}

/// Start a long-lived job and describe it in the terms the next call needs: the id to ask about
/// it with, and where its output is going.
pub(super) async fn start_background(
    command: &str,
    cwd: &str,
    workspace: Option<&Path>,
    policy: &SandboxPolicy,
) -> String {
    match start(command, cwd, workspace, policy).await {
        Ok(job) => format!(
            "shell: started background job {} (pid {})\n  log: {}\n  \
             Check on it with shell_job: {{\"action\": \"log\", \"id\": {}}}, stop it with \
             {{\"action\": \"stop\", \"id\": {}}}. It keeps running until stopped.",
            job.pid,
            job.pid,
            job.log.display(),
            job.pid,
            job.pid
        ),
        Err(error) => format!("shell: could not start background job: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// The whole point: the job is still running after `start` returned, and it is NOT in this
    /// process's group, so the foreground group kill could never have reached it.
    #[tokio::test]
    #[cfg(unix)]
    async fn a_background_job_outlives_the_call_and_leaves_the_callers_process_group() {
        let dir = workspace();
        let cwd = dir.path().display().to_string();
        let job = start(
            "sleep 30",
            &cwd,
            Some(dir.path()),
            &SandboxPolicy::default(),
        )
        .await
        .expect("start");

        assert!(
            job.is_running(),
            "the job must survive the call that started it"
        );
        let ours = unsafe { libc::getpgid(0) };
        let theirs = unsafe { libc::getpgid(job.pid as i32) };
        assert_ne!(
            theirs, ours,
            "a background job in the caller's process group would die with the next group kill"
        );

        stop(Some(dir.path()), &cwd, job.pid).await.unwrap();
        assert!(!job.is_running(), "stop must actually stop it");
    }

    /// A job started by one call has to be findable by the next one, which means the record — not
    /// in-memory state — is the source of truth.
    #[tokio::test]
    #[cfg(unix)]
    async fn a_job_is_findable_from_a_later_call_by_its_id_alone() {
        let dir = workspace();
        let cwd = dir.path().display().to_string();
        let job = start(
            "echo hello-from-the-job; sleep 30",
            &cwd,
            Some(dir.path()),
            &SandboxPolicy::default(),
        )
        .await
        .expect("start");

        let found = get(Some(dir.path()), &cwd, job.pid).expect("job record on disk");
        assert_eq!(found.pid, job.pid);
        assert!(list(Some(dir.path()), &cwd)
            .iter()
            .any(|j| j.pid == job.pid));

        // The log is a file, not a pipe, so output is readable while the job is still running.
        for _ in 0..40 {
            if tail(&found.log, 10)
                .unwrap_or_default()
                .contains("hello-from-the-job")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(
            tail(&found.log, 10).unwrap().contains("hello-from-the-job"),
            "a running job's output must be readable from its log"
        );
        stop(Some(dir.path()), &cwd, job.pid).await.unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn a_finished_job_reports_its_exit_code_rather_than_only_being_gone() {
        let dir = workspace();
        let cwd = dir.path().display().to_string();
        let job = start("exit 3", &cwd, Some(dir.path()), &SandboxPolicy::default())
            .await
            .expect("start");
        for _ in 0..40 {
            if let Some(found) = get(Some(dir.path()), &cwd, job.pid) {
                if found.exit == Some(3) {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("the exit code of a finished job was never recorded");
    }

    #[test]
    fn a_tail_of_a_short_log_is_the_whole_log() {
        let dir = workspace();
        let log = dir.path().join("j.log");
        std::fs::write(&log, "one\ntwo\nthree\n").unwrap();
        assert_eq!(tail(&log, 40).unwrap(), "one\ntwo\nthree");
        assert_eq!(tail(&log, 2).unwrap(), "two\nthree");
    }

    #[test]
    fn the_survivor_note_names_what_it_killed_and_what_to_do_instead() {
        let note = survivor_note(&[(42, "emulator".into()), (43, "adb".into())]);
        assert!(note.contains("emulator(42)"), "{note}");
        assert!(note.contains("background:true"), "{note}");
    }
}
