//! Claude Code's `statusLine.command` contract: run a user command with session JSON on stdin and
//! show the first line of its stdout. The command is spawned detached, so a slow or hung script
//! only leaves the previous output on screen; it can never stall the render loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Minimum gap between runs, matching Claude Code's update debounce.
const DEBOUNCE: Duration = Duration::from_millis(300);
const TIMEOUT: Duration = Duration::from_secs(3);
const FALLBACK_CONTEXT_WINDOW: u64 = 200_000;

/// Session state the payload is built from.
pub(crate) struct StatuslineInput<'a> {
    pub session_id: &'a str,
    pub model: &'a str,
    pub cwd: &'a str,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub session_in: u64,
    pub session_out: u64,
    pub context_tokens: u64,
    pub context_limit: Option<u32>,
}

/// The JSON Claude Code feeds a statusline command. Forge keeps its transcript in its store, not a
/// jsonl file, so `transcript_path` is empty (as it is for CC-compatible hooks); line counts are
/// not tracked and read as 0.
pub(crate) fn payload(i: &StatuslineInput<'_>) -> serde_json::Value {
    let window = i
        .context_limit
        .map_or(FALLBACK_CONTEXT_WINDOW, u64::from)
        .max(1);
    let used_pct = (i.context_tokens as f64 / window as f64 * 100.0).min(100.0);
    let current_usage = if i.context_tokens == 0 {
        serde_json::Value::Null
    } else {
        serde_json::json!({
            "input_tokens": i.context_tokens,
            "output_tokens": 0,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
        })
    };
    serde_json::json!({
        "hook_event_name": "Status",
        "session_id": i.session_id,
        "transcript_path": "",
        "cwd": i.cwd,
        "model": { "id": i.model, "display_name": i.model },
        "workspace": { "current_dir": i.cwd, "project_dir": i.cwd },
        "version": env!("CARGO_PKG_VERSION"),
        "output_style": { "name": "default" },
        "cost": {
            "total_cost_usd": i.cost_usd,
            "total_duration_ms": i.duration_ms,
            "total_api_duration_ms": 0,
            "total_lines_added": 0,
            "total_lines_removed": 0,
        },
        "context_window": {
            "total_input_tokens": i.session_in,
            "total_output_tokens": i.session_out,
            "context_window_size": window,
            "used_percentage": used_pct,
            "remaining_percentage": 100.0 - used_pct,
            "current_usage": current_usage,
        },
        "exceeds_200k_tokens": i.context_tokens > 200_000,
    })
}

/// Decides when a new run is due: the payload (ignoring the ever-ticking duration) changed, the
/// debounce window has passed, and the previous run finished.
#[derive(Default)]
pub(crate) struct Scheduler {
    last_start: Option<Instant>,
    last_key: Option<String>,
    in_flight: Arc<AtomicBool>,
}

impl Scheduler {
    /// Returns the in-flight flag to clear when the run ends, or `None` when no run is due.
    pub(crate) fn due(
        &mut self,
        payload: &serde_json::Value,
        now: Instant,
    ) -> Option<Arc<AtomicBool>> {
        let mut keyed = payload.clone();
        keyed["cost"]["total_duration_ms"] = 0.into();
        let key = keyed.to_string();
        if self.last_key.as_deref() == Some(key.as_str())
            || self.in_flight.load(Ordering::Relaxed)
            || self
                .last_start
                .is_some_and(|t| now.duration_since(t) < DEBOUNCE)
        {
            return None;
        }
        self.last_key = Some(key);
        self.last_start = Some(now);
        self.in_flight.store(true, Ordering::Relaxed);
        Some(self.in_flight.clone())
    }
}

/// Run `command` with `payload` on stdin; the first stdout line on success, `None` on failure,
/// non-zero exit, or timeout.
pub(crate) async fn run_command(command: &str, payload: &serde_json::Value) -> Option<String> {
    use tokio::io::AsyncWriteExt;
    let (sh, flag) = super::shell_widget_shell();
    let mut child = tokio::process::Command::new(sh)
        .arg(flag)
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let body = payload.to_string();
    let work = async move {
        // A script that never reads stdin closes the pipe early; that is not a failure.
        let _ = stdin.write_all(body.as_bytes()).await;
        drop(stdin);
        child.wait_with_output().await
    };
    let out = tokio::time::timeout(TIMEOUT, work).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.lines().next().unwrap_or("").trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> StatuslineInput<'static> {
        StatuslineInput {
            session_id: "s1",
            model: "kimi-k2",
            cwd: "/w",
            cost_usd: 0.5,
            duration_ms: 1234,
            session_in: 10,
            session_out: 5,
            context_tokens: 50_000,
            context_limit: Some(100_000),
        }
    }

    #[test]
    fn payload_has_the_claude_code_fields() {
        let p = payload(&input());
        assert_eq!(p["session_id"], "s1");
        assert_eq!(p["model"]["display_name"], "kimi-k2");
        assert_eq!(p["workspace"]["project_dir"], "/w");
        assert_eq!(p["cost"]["total_cost_usd"], 0.5);
        assert_eq!(p["cost"]["total_duration_ms"], 1234);
        assert_eq!(p["cost"]["total_lines_added"], 0);
        assert_eq!(p["context_window"]["context_window_size"], 100_000);
        assert_eq!(p["context_window"]["used_percentage"], 50.0);
        assert_eq!(p["exceeds_200k_tokens"], false);
        assert!(p["version"].is_string() && p["transcript_path"].is_string());
    }

    #[test]
    fn fresh_session_has_null_usage_and_default_window() {
        let mut i = input();
        i.context_tokens = 0;
        i.context_limit = None;
        let p = payload(&i);
        assert!(p["context_window"]["current_usage"].is_null());
        assert_eq!(p["context_window"]["context_window_size"], 200_000);
        i.context_tokens = 250_000;
        assert_eq!(payload(&i)["exceeds_200k_tokens"], true);
    }

    #[test]
    fn scheduler_debounces_and_ignores_duration_ticks() {
        let mut s = Scheduler::default();
        let t0 = Instant::now();
        let p = payload(&input());
        let flag = s.due(&p, t0).expect("first run is due");
        flag.store(false, Ordering::Relaxed);

        let mut ticked = input();
        ticked.duration_ms = 99_999;
        assert!(s
            .due(&payload(&ticked), t0 + Duration::from_secs(5))
            .is_none());

        let mut changed = input();
        changed.cost_usd = 0.7;
        assert!(
            s.due(&payload(&changed), t0 + Duration::from_millis(100))
                .is_none(),
            "inside the debounce window"
        );
        assert!(s
            .due(&payload(&changed), t0 + Duration::from_millis(400))
            .is_some());
    }

    #[test]
    fn scheduler_waits_for_the_running_command() {
        let mut s = Scheduler::default();
        let t0 = Instant::now();
        let flag = s.due(&payload(&input()), t0).unwrap();
        let mut changed = input();
        changed.cost_usd = 9.0;
        let later = t0 + Duration::from_secs(1);
        assert!(s.due(&payload(&changed), later).is_none());
        flag.store(false, Ordering::Relaxed);
        assert!(s.due(&payload(&changed), later).is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_command_with_json_on_stdin_and_takes_first_line() {
        let p = payload(&input());
        let out = run_command(
            r#"sed 's/.*"display_name":"\([^"]*\)".*/\1/'; echo; echo second"#,
            &p,
        )
        .await;
        assert_eq!(out.as_deref(), Some("kimi-k2"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failure_and_timeout_yield_none() {
        let p = payload(&input());
        assert_eq!(run_command("exit 3", &p).await, None);
        assert_eq!(run_command("true", &p).await.as_deref(), Some(""));
    }
}
