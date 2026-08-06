//! User-defined autonomous quality gates (prime-agent parity — docs/features/autonomous-gates.md).
//!
//! A gate is a shell command that must exit 0 before `/loop`/`/goal` are allowed to finish. Gates
//! run in order; the first failure is fed back to the model as the next iteration's prompt instead
//! of ending the run. A gate is retried up to a per-gate limit before the whole run stops with a
//! "gate exhausted" reason (never reported as success). Re-running an unchanged workspace against
//! the same failing gate wastes an iteration on a certain repeat, so a gate whose last failure
//! fingerprint matches the current one is skipped — it still counts as another attempt.

use std::process::Stdio;
use std::time::Duration;

use sha2::Digest as _;
use tokio::process::{Child, Command};

/// Default per-gate retry budget before a run is declared "gate exhausted".
pub(crate) const DEFAULT_GATE_RETRIES: u32 = 3;
/// Default per-gate wall-clock timeout; the whole process tree is killed on expiry.
pub(crate) const DEFAULT_GATE_TIMEOUT: Duration = Duration::from_secs(300);
/// Bounded gate output handed back to the model on failure — the last N characters of combined
/// stdout+stderr, never the full (possibly huge) log.
const BOUNDED_OUTPUT_CHARS: usize = 4000;
/// Grace period between SIGTERM and SIGKILL on a gate timeout (mirrors forge-tools::shell's
/// process-group kill idiom).
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_secs(2);

/// A single user-defined quality gate: a shell command that must exit 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateSpec {
    pub(crate) cmd: String,
}

/// Per-gate run state carried across iterations of one autonomous run: how many times it has
/// been attempted, the workspace fingerprint at its last failure, and the bounded output from
/// that failure (replayed verbatim when a fingerprint match skips the rerun — the model still
/// needs to see what failed).
pub(crate) struct GateState {
    pub(crate) spec: GateSpec,
    pub(crate) attempts: u32,
    pub(crate) last_failure_fingerprint: Option<String>,
    last_bounded_output: String,
}

impl GateState {
    pub(crate) fn new(spec: GateSpec) -> Self {
        Self {
            spec,
            attempts: 0,
            last_failure_fingerprint: None,
            last_bounded_output: String::new(),
        }
    }
}

/// Per-gate retry/timeout policy — the same for every gate in a run (no per-gate override yet).
pub(crate) struct GateConfig {
    pub(crate) retries: u32,
    pub(crate) timeout: Duration,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            retries: DEFAULT_GATE_RETRIES,
            timeout: DEFAULT_GATE_TIMEOUT,
        }
    }
}

/// Result of running the full gate set once.
pub(crate) enum GateOutcome {
    /// Every gate passed (or was already known-passing) — the run may finish.
    AllPassed,
    /// `gate_index` failed within its retry budget; `bounded_output` is what the model should see.
    Failed {
        gate_index: usize,
        bounded_output: String,
    },
    /// `gate_index` exceeded its retry budget — the run must stop, not succeed.
    Exhausted { gate_index: usize },
}

/// Run every gate in order, stopping at the first failure (or exhaustion). A gate whose last
/// failure fingerprint matches `workspace_fingerprint` is NOT re-executed — nothing has changed
/// since it failed, so re-running it would just burn the retry budget on a certain repeat; it
/// still counts as another attempt against that budget. An empty `workspace_fingerprint` (not a
/// git repo — see [`workspace_fingerprint`]) always reruns, since "unchanged" can't be verified.
pub(crate) async fn run_gates(
    states: &mut [GateState],
    cfg: &GateConfig,
    workspace_fingerprint: &str,
) -> GateOutcome {
    for (gate_index, state) in states.iter_mut().enumerate() {
        let skip_rerun = !workspace_fingerprint.is_empty()
            && state.last_failure_fingerprint.as_deref() == Some(workspace_fingerprint);
        let (passed, bounded_output) = if skip_rerun {
            (false, state.last_bounded_output.clone())
        } else {
            let (ok, output) = run_gate_command(&state.spec.cmd, cfg.timeout).await;
            let bounded = bound_output(&output, BOUNDED_OUTPUT_CHARS);
            if !ok {
                state.last_failure_fingerprint = Some(workspace_fingerprint.to_string());
                state.last_bounded_output = bounded.clone();
            }
            (ok, bounded)
        };
        if passed {
            continue;
        }
        state.attempts += 1;
        if state.attempts > cfg.retries {
            return GateOutcome::Exhausted { gate_index };
        }
        return GateOutcome::Failed {
            gate_index,
            bounded_output,
        };
    }
    GateOutcome::AllPassed
}

/// Truncate `s` to at most `max_chars` characters, keeping the TAIL (the most recent output
/// matters most for diagnosing a failure) and marking that truncation happened.
fn bound_output(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let dropped = total - max_chars;
    let tail: String = s.chars().skip(dropped).collect();
    format!("…[truncated {dropped} earlier chars]…\n{tail}")
}

/// Execute `cmd` via the OS shell, capture combined stdout+stderr, and kill the whole process
/// tree if it outruns `timeout`. gates.rs can't reuse forge-tools' shell runtime directly (it's a
/// private tool-execution path in a different crate) so this mirrors its process-group kill idiom.
async fn run_gate_command(cmd: &str, timeout: Duration) -> (bool, String) {
    let (shell, flag) = shell_invocation();
    let mut command = Command::new(shell);
    command
        .arg(flag)
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("gate failed to start: {e}")),
    };
    let pgid = child.id().map(|id| id as i32);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_task = tokio::spawn(read_to_end_capped(stdout));
    let err_task = tokio::spawn(read_to_end_capped(stderr));

    let (success, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => (status.success(), false),
        Ok(Err(e)) => return (false, format!("gate wait failed: {e}")),
        Err(_) => {
            kill_tree(&mut child, pgid).await;
            (false, true)
        }
    };

    let out_bytes = out_task.await.unwrap_or_default();
    let err_bytes = err_task.await.unwrap_or_default();
    let mut combined = String::from_utf8_lossy(&out_bytes).into_owned();
    if !err_bytes.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&err_bytes));
    }
    if timed_out {
        combined.push_str(&format!(
            "\n[gate timed out after {}s — killed]",
            timeout.as_secs()
        ));
    }
    (success, combined)
}

/// Read a child pipe to EOF, capped so a runaway gate can't exhaust memory.
async fn read_to_end_capped<R: tokio::io::AsyncRead + Unpin>(stream: Option<R>) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    const CAP: usize = 256 * 1024;
    let Some(mut stream) = stream else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if buf.len() < CAP {
                    let take = n.min(CAP - buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
    buf
}

/// Kill the gate's whole process tree on timeout: SIGTERM→grace→SIGKILL on the process group
/// (Unix); `taskkill /F /T` on Windows. The tree matters because `sh -c`/`cmd /C` spawn the real
/// command as a child — killing only the shell would leave it running and hold the output pipes
/// open.
async fn kill_tree(child: &mut Child, pgid: Option<i32>) {
    #[cfg(unix)]
    {
        if let Some(pg) = pgid {
            unsafe { libc::kill(-pg, libc::SIGTERM) };
            tokio::time::sleep(KILL_GRACE).await;
            unsafe { libc::kill(-pg, libc::SIGKILL) };
        }
        let _ = child.wait().await;
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        if let Some(pid) = child.id() {
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

/// The OS shell + its "run this command string" flag: `sh -c` on Unix, `cmd /C` on Windows.
fn shell_invocation() -> (&'static str, &'static str) {
    #[cfg(windows)]
    {
        ("cmd", "/C")
    }
    #[cfg(not(windows))]
    {
        ("sh", "-c")
    }
}

/// Fingerprint of the working tree's uncommitted state: a hash of `git status --porcelain` +
/// `git diff HEAD`. Used to decide whether a gate that just failed is worth re-running — an
/// unchanged workspace can't have started passing. Returns `""` outside a git repo (or on any git
/// failure), which [`run_gates`] treats as "always rerun" since "unchanged" can't be verified.
pub(crate) async fn workspace_fingerprint(cwd: &std::path::Path) -> String {
    let status = capture_git(cwd, &["status", "--porcelain"]).await;
    let diff = capture_git(cwd, &["diff", "HEAD"]).await;
    match (status, diff) {
        (Some(s), Some(d)) => {
            let mut combined = s;
            combined.push('\n');
            combined.push_str(&d);
            hex::encode(sha2::Sha256::digest(combined.as_bytes()))
        }
        _ => String::new(),
    }
}

async fn capture_git(cwd: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("git").args(args).current_dir(cwd).output(),
    )
    .await
    .ok()?
    .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(retries: u32) -> GateConfig {
        GateConfig {
            retries,
            timeout: Duration::from_secs(10),
        }
    }

    fn gate(cmd: &str) -> GateState {
        GateState::new(GateSpec {
            cmd: cmd.to_string(),
        })
    }

    #[test]
    fn bound_output_passes_short_output_through_unchanged() {
        assert_eq!(bound_output("short", 4000), "short");
    }

    #[test]
    fn bound_output_keeps_the_tail_and_marks_truncation() {
        let long = format!("{}{}", "a".repeat(4900), "TAIL_MARKER");
        let bounded = bound_output(&long, 4000);
        assert!(bounded.contains("truncated"), "marks truncation: {bounded}");
        assert!(
            bounded.ends_with("TAIL_MARKER"),
            "keeps the tail: {bounded}"
        );
        assert!(bounded.len() < long.len());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn all_passing_gates_return_all_passed() {
        let mut states = vec![gate("true"), gate("true")];
        let outcome = run_gates(&mut states, &cfg(3), "fp").await;
        assert!(matches!(outcome, GateOutcome::AllPassed));
        assert_eq!(states[0].attempts, 0);
        assert_eq!(states[1].attempts, 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn first_failing_gate_stops_before_the_next_runs() {
        let mut states = vec![gate("false"), gate("true")];
        let outcome = run_gates(&mut states, &cfg(3), "fp").await;
        match outcome {
            GateOutcome::Failed { gate_index, .. } => assert_eq!(gate_index, 0),
            _ => panic!("expected Failed"),
        }
        assert_eq!(states[0].attempts, 1);
        assert_eq!(
            states[1].attempts, 0,
            "second gate must not run after the first fails"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_output_from_a_failing_gate_contains_its_stderr() {
        let mut states = vec![gate("sh -c 'echo boom >&2; exit 1'")];
        let outcome = run_gates(&mut states, &cfg(3), "fp").await;
        match outcome {
            GateOutcome::Failed { bounded_output, .. } => {
                assert!(bounded_output.contains("boom"), "got: {bounded_output}");
            }
            _ => panic!("expected Failed"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exceeding_retries_reports_exhausted() {
        let mut states = vec![gate("false")];
        let c = cfg(1);
        let first = run_gates(&mut states, &c, "fp1").await;
        assert!(matches!(first, GateOutcome::Failed { gate_index: 0, .. }));
        // A different fingerprint forces a rerun; the second failure exceeds retries=1.
        let second = run_gates(&mut states, &c, "fp2").await;
        assert!(matches!(second, GateOutcome::Exhausted { gate_index: 0 }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unchanged_workspace_skips_rerun_but_still_counts_as_a_failed_attempt() {
        let marker = std::env::temp_dir().join(format!(
            "forge-gate-marker-unchanged-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("echo x >> {} ; exit 1", marker.display());
        let mut states = vec![gate(&cmd)];
        let c = cfg(5);
        let _ = run_gates(&mut states, &c, "same-fp").await;
        let _ = run_gates(&mut states, &c, "same-fp").await;
        let lines = std::fs::read_to_string(&marker).unwrap_or_default();
        assert_eq!(
            lines.lines().count(),
            1,
            "gate must not rerun when the workspace fingerprint is unchanged"
        );
        assert_eq!(
            states[0].attempts, 2,
            "attempts still advance without a rerun"
        );
        let _ = std::fs::remove_file(&marker);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn changed_workspace_fingerprint_reruns_the_gate() {
        let marker =
            std::env::temp_dir().join(format!("forge-gate-marker-changed-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("echo x >> {} ; exit 1", marker.display());
        let mut states = vec![gate(&cmd)];
        let c = cfg(5);
        let _ = run_gates(&mut states, &c, "fp-a").await;
        let _ = run_gates(&mut states, &c, "fp-b").await;
        let lines = std::fs::read_to_string(&marker).unwrap_or_default();
        assert_eq!(
            lines.lines().count(),
            2,
            "a changed fingerprint must rerun the gate"
        );
        let _ = std::fs::remove_file(&marker);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn empty_fingerprint_always_reruns() {
        let marker =
            std::env::temp_dir().join(format!("forge-gate-marker-empty-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("echo x >> {} ; exit 1", marker.display());
        let mut states = vec![gate(&cmd)];
        let c = cfg(5);
        let _ = run_gates(&mut states, &c, "").await;
        let _ = run_gates(&mut states, &c, "").await;
        let lines = std::fs::read_to_string(&marker).unwrap_or_default();
        assert_eq!(
            lines.lines().count(),
            2,
            "an empty (non-git) fingerprint must always rerun"
        );
        let _ = std::fs::remove_file(&marker);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn gate_command_times_out_and_is_killed() {
        let start = std::time::Instant::now();
        let (ok, output) = run_gate_command("sleep 30", Duration::from_secs(1)).await;
        assert!(!ok);
        assert!(output.contains("timed out"), "got: {output}");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "must not wait for the full sleep"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_fingerprint_reflects_uncommitted_changes() {
        let dir = std::env::temp_dir().join(format!("forge-gate-fp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap()
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@t.com"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);

        let fp1 = workspace_fingerprint(&dir).await;
        assert!(!fp1.is_empty());
        let fp2 = workspace_fingerprint(&dir).await;
        assert_eq!(fp1, fp2, "stable for an unchanged workspace");

        std::fs::write(dir.join("f.txt"), "b").unwrap();
        let fp3 = workspace_fingerprint(&dir).await;
        assert_ne!(fp1, fp3, "changes must change the fingerprint");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn workspace_fingerprint_is_empty_outside_a_git_repo() {
        let dir = std::env::temp_dir().join(format!("forge-gate-nogit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fp = workspace_fingerprint(&dir).await;
        assert_eq!(fp, "");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_digest_is_reachable() {
        // Smoke-test the exact call `workspace_fingerprint` makes so a broken import fails fast.
        let digest = sha2::Sha256::digest(b"x");
        assert_eq!(hex::encode(digest).len(), 64);
    }
}
