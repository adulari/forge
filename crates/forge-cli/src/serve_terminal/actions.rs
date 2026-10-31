//! Project actions (`[[actions]]`) as daemon terminals.
//!
//! An action is a PTY like any other, started with a command line instead of an interactive
//! shell and keyed `act-<action id>` so a client can tell it from `term-N`. Output, attach and
//! reconnect therefore come from the terminal machinery; this module adds start, stop (which
//! takes the whole process group down, not just the shell) and a running summary.

use super::*;
use crate::serve_terminal_util::action_terminal_id;

/// Action terminals one session may hold, counted apart from its interactive shells.
const MAX_ACTION_TERMINALS_PER_SESSION: usize = 8;
const KILL_GRACE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The arguments that make `shell` run one command line and then exit.
pub(super) fn run_args(shell: &str, line: &str) -> Vec<String> {
    let name = std::path::Path::new(shell)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if cfg!(windows) {
        return match name.as_str() {
            "powershell" | "pwsh" => vec!["-NoLogo".into(), "-Command".into(), line.into()],
            _ => vec!["/C".into(), line.into()],
        };
    }
    vec!["-lc".into(), line.into()]
}

/// Ask the action's process group to stop, then make sure it did. The pty makes the child a
/// session leader, so its pid is the group id and a dev server's children are in it.
pub(super) fn terminate_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.filter(|pid| *pid > 1) {
        let group = -(pid as i32);
        // SAFETY: kill(2) with a negative pid signals a process group; no memory is touched.
        unsafe { libc::kill(group, libc::SIGTERM) };
        tokio::spawn(async move {
            tokio::time::sleep(KILL_GRACE).await;
            // SAFETY: as above. ESRCH when the group already exited is the expected outcome.
            unsafe { libc::kill(group, libc::SIGKILL) };
        });
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Handles `restart` for a `/ws/terminal` request and gates `act-` terminals: an action tab never
/// falls back to an interactive shell. A restart re-runs the action from its config; attaching to
/// one that is gone is refused. `None` lets the request proceed.
pub(super) async fn prepare_terminal(
    state: &crate::serve::DaemonState,
    key: &TerminalKey,
    restart: bool,
) -> Option<axum::response::Response> {
    use crate::serve::err_response;
    let Some(action) = action_of_terminal(&key.terminal_id) else {
        if restart {
            state.terminals.close(key).await;
        }
        return None;
    };
    if restart {
        let restarted = crate::serve_project_actions::restart(state, &key.session_id, action).await;
        return restarted
            .err()
            .map(|(status, message)| err_response(status, &message));
    }
    if state.terminals.terminals.lock().await.contains_key(key) {
        return None;
    }
    Some(err_response(
        axum::http::StatusCode::NOT_FOUND,
        "that action is not running; run it again from the Run menu",
    ))
}

/// One action's state on a session.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ActionRun {
    pub session: String,
    pub action: String,
    pub terminal_id: String,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    pub updated_at_ms: u64,
}

impl TerminalRegistry {
    /// Start `line` in `cwd` as the terminal for `action`. Already running is a no-op; a finished
    /// run is replaced.
    pub(crate) async fn start_action(
        &self,
        session_id: &str,
        action: &str,
        cwd: String,
        line: &str,
        host_limit: u32,
    ) -> Result<(), String> {
        let key = TerminalKey {
            session_id: session_id.to_string(),
            terminal_id: action_terminal_id(action),
        };
        let mut terminals = self.terminals.lock().await;
        if let Some(existing) = terminals.get(&key) {
            if existing.status() != TerminalStatus::Exited {
                return Ok(());
            }
            terminals.remove(&key);
        }
        let open = terminals
            .values()
            .filter(|handle| handle.status() != TerminalStatus::Exited)
            .count();
        if let Some(message) = crate::host_caps::terminals_refusal(open, host_limit) {
            return Err(message);
        }
        let held = terminals
            .keys()
            .filter(|candidate| {
                candidate.session_id == session_id
                    && action_of_terminal(&candidate.terminal_id).is_some()
            })
            .count();
        if held >= MAX_ACTION_TERMINALS_PER_SESSION {
            return Err(format!(
                "a session may run at most {MAX_ACTION_TERMINALS_PER_SESSION} actions at once; \
                 stop one first"
            ));
        }
        let handle = spawn_terminal(key.clone(), cwd, 100, 28, Some(line))?;
        terminals.insert(key, handle);
        Ok(())
    }

    /// Whether the session still has an action running, which keeps it loaded so its Ports and
    /// Stop stay reachable.
    pub(crate) async fn has_running_action(&self, session_id: &str) -> bool {
        self.action_runs(Some(session_id))
            .await
            .iter()
            .any(|run| run.running)
    }

    /// Stop an action and discard its terminal. False when it was not there.
    pub(crate) async fn stop_action(&self, session_id: &str, action: &str) -> bool {
        let key = TerminalKey {
            session_id: session_id.to_string(),
            terminal_id: action_terminal_id(action),
        };
        self.close(&key).await
    }

    /// Every action terminal, for one session or (None) across all of them.
    pub(crate) async fn action_runs(&self, session_id: Option<&str>) -> Vec<ActionRun> {
        let terminals = self.terminals.lock().await;
        let mut runs: Vec<_> = terminals
            .values()
            .filter(|handle| session_id.is_none_or(|id| handle.key.session_id == id))
            .filter_map(|handle| {
                let action = action_of_terminal(&handle.key.terminal_id)?;
                Some(ActionRun {
                    session: handle.key.session_id.clone(),
                    action: action.to_string(),
                    terminal_id: handle.key.terminal_id.clone(),
                    running: handle.status() != TerminalStatus::Exited,
                    exit_code: handle.exit_code(),
                    updated_at_ms: handle.updated_at_ms.load(Ordering::Acquire),
                })
            })
            .collect();
        runs.sort_by(|a, b| (&a.session, &a.action).cmp(&(&b.session, &b.action)));
        runs
    }

    /// Pids of the shells and actions a session holds, with the action id for the latter.
    pub(crate) async fn session_roots(&self, session_id: &str) -> Vec<(u32, Option<String>)> {
        self.terminals
            .lock()
            .await
            .values()
            .filter(|handle| handle.key.session_id == session_id)
            .filter(|handle| handle.status() != TerminalStatus::Exited)
            .filter_map(|handle| {
                let action = action_of_terminal(&handle.key.terminal_id).map(str::to_string);
                Some((handle.pid?, action))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive(pid: i32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|stat| !stat.contains(") Z "))
            .unwrap_or(false)
    }

    async fn wait_for<F: Fn() -> bool>(check: F) -> bool {
        for _ in 0..80 {
            if check() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        false
    }

    #[test]
    fn run_args_hand_the_line_to_a_login_shell() {
        if !cfg!(windows) {
            assert_eq!(run_args("/bin/zsh", "npm run dev"), ["-lc", "npm run dev"]);
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stopping_an_action_kills_its_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("child.pid");
        let registry = TerminalRegistry::new();
        let line = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
        registry
            .start_action("s1", "dev", dir.path().display().to_string(), &line, 0)
            .await
            .unwrap();

        assert!(
            wait_for(|| std::fs::read_to_string(&pidfile).is_ok_and(|p| !p.trim().is_empty()))
                .await
        );
        let child: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(alive(child));

        let runs = registry.action_runs(Some("s1")).await;
        assert_eq!(runs.len(), 1);
        assert!(runs[0].running);
        assert_eq!(runs[0].action, "dev");
        assert_eq!(registry.session_roots("s1").await.len(), 1);

        assert!(registry.stop_action("s1", "dev").await);
        assert!(
            wait_for(|| !alive(child)).await,
            "the grandchild survived Stop"
        );
        assert!(registry.action_runs(Some("s1")).await.is_empty());
        assert!(!registry.stop_action("s1", "dev").await);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_finished_action_reports_its_exit_code_and_can_run_again() {
        let dir = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::new();
        let cwd = dir.path().display().to_string();
        registry
            .start_action("s1", "fail", cwd.clone(), "exit 3", 0)
            .await
            .unwrap();
        let mut done = None;
        for _ in 0..80 {
            let runs = registry.action_runs(Some("s1")).await;
            if runs.first().is_some_and(|run| !run.running) {
                done = runs.first().cloned();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(done.expect("the action should finish").exit_code, Some(3));
        registry
            .start_action("s1", "fail", cwd, "sleep 30", 0)
            .await
            .unwrap();
        assert!(registry.action_runs(None).await[0].running);
        registry.stop_action("s1", "fail").await;
    }

    #[tokio::test]
    async fn the_per_session_action_cap_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let registry = TerminalRegistry::new();
        let cwd = dir.path().display().to_string();
        for n in 0..MAX_ACTION_TERMINALS_PER_SESSION {
            registry
                .start_action("s1", &format!("a{n}"), cwd.clone(), "sleep 30", 0)
                .await
                .unwrap();
        }
        let refused = registry
            .start_action("s1", "extra", cwd, "sleep 30", 0)
            .await;
        assert!(refused.unwrap_err().contains("at most"));
        for n in 0..MAX_ACTION_TERMINALS_PER_SESSION {
            registry.stop_action("s1", &format!("a{n}")).await;
        }
    }

    #[tokio::test]
    async fn an_action_terminal_never_becomes_a_shell() {
        use std::sync::Arc;
        let state = crate::serve::DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: "/tmp".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        };
        let key = |id: &str| TerminalKey {
            session_id: "s1".into(),
            terminal_id: id.into(),
        };
        let refused = prepare_terminal(&state, &key("act-dev"), false).await;
        assert_eq!(
            refused.expect("a missing action tab is refused").status(),
            axum::http::StatusCode::NOT_FOUND
        );
        let restart = prepare_terminal(&state, &key("act-dev"), true).await;
        assert!(
            restart.is_some(),
            "a restart with no such session or action is refused"
        );
        assert!(prepare_terminal(&state, &key("term-1"), true)
            .await
            .is_none());
    }
}
