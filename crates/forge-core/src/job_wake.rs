//! Background-job completion notices (Claude-Code `task-notification` parity).
//!
//! A job the model started with `shell{background:true}` reports its exit here, and the model hears
//! about it without having to poll `shell_job`:
//!
//! * mid-turn, [`Session::inject_steers`](crate::Session) drains the inbox at the same boundaries a
//!   queued user prompt joins the conversation (after tool results, and before a response would end
//!   the turn);
//! * idle, the surface's periodic check (`try_deliver_due_heartbeats`, shared by the TUI loop and the
//!   daemon driver) claims a coalesced prompt and starts a turn, rate-limited so a flapping job
//!   cannot burn tokens unattended.
//!
//! Only jobs started by this session's model calls are reported: the sink is scoped around that one
//! tool call (`forge_tools::scope_job_exit_sink`), not installed process-wide.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forge_tools::JobExit;
use forge_types::{Message, Role};

/// Pending notices kept; the oldest is dropped (and counted) beyond this.
const MAX_PENDING: usize = 20;
/// Idle notices arriving within this long of each other become one turn, not several.
const COALESCE: Duration = Duration::from_millis(1500);
/// Automatic idle turns allowed per [`WINDOW`]. The rest wait for the next user turn to drain them.
const MAX_AUTO_TURNS: usize = 6;
const WINDOW: Duration = Duration::from_secs(600);
/// Characters of a job's command and log tail quoted in a notice.
const COMMAND_CHARS: usize = 100;
const TAIL_CHARS: usize = 600;

#[derive(Default)]
struct State {
    pending: VecDeque<(Instant, String)>,
    dropped: usize,
    auto_turns: VecDeque<Instant>,
}

/// Cheap-to-clone handle; the session holds one and each job-exit sink holds another.
#[derive(Clone, Default)]
pub struct JobWake {
    state: Arc<Mutex<State>>,
}

impl JobWake {
    /// Queue a notice for a job that just exited.
    pub fn push(&self, exit: &JobExit) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if st.pending.len() >= MAX_PENDING {
            st.pending.pop_front();
            st.dropped += 1;
        }
        st.pending.push_back((Instant::now(), render(exit)));
    }

    /// Everything pending, for delivery into a running turn. Never rate-limited: the model is
    /// already working and this costs no extra turn.
    pub(crate) fn drain(&self) -> Vec<String> {
        let Ok(mut st) = self.state.lock() else {
            return Vec::new();
        };
        take_all(&mut st)
    }

    /// For an idle session: one prompt covering every pending notice, or `None` if there is
    /// nothing, the newest notice is still within the coalescing window, or the automatic-turn
    /// budget for this window is spent (the notices stay queued for the next user turn).
    pub fn claim_idle_prompt(&self) -> Option<String> {
        let mut st = self.state.lock().ok()?;
        let now = Instant::now();
        let newest = st.pending.back()?.0;
        if now.duration_since(newest) < COALESCE {
            return None;
        }
        while st
            .auto_turns
            .front()
            .is_some_and(|t| now.duration_since(*t) >= WINDOW)
        {
            st.auto_turns.pop_front();
        }
        if st.auto_turns.len() >= MAX_AUTO_TURNS {
            return None;
        }
        st.auto_turns.push_back(now);
        Some(take_all(&mut st).join("\n\n"))
    }

    pub fn has_pending(&self) -> bool {
        self.state.lock().is_ok_and(|st| !st.pending.is_empty())
    }
}

fn take_all(st: &mut State) -> Vec<String> {
    let mut out: Vec<String> = st.pending.drain(..).map(|(_, text)| text).collect();
    if st.dropped > 0 {
        out.insert(
            0,
            format!(
                "[{} earlier background-job notice(s) were dropped; check `shell_job` list]",
                st.dropped
            ),
        );
        st.dropped = 0;
    }
    out
}

fn render(exit: &JobExit) -> String {
    let command =
        forge_types::truncate_ellipsis(exit.command.lines().next().unwrap_or(""), COMMAND_CHARS);
    let mut text = format!(
        "[background job {} `{command}` exited {}]",
        exit.pid, exit.code
    );
    let tail = exit.tail.trim();
    if !tail.is_empty() {
        let start = tail.len().saturating_sub(TAIL_CHARS);
        let start = (start..=tail.len())
            .find(|i| tail.is_char_boundary(*i))
            .unwrap_or(tail.len());
        text.push_str("\nlast lines:\n");
        text.push_str(&tail[start..]);
    }
    text
}

impl crate::Session {
    /// Background-job completion notices for this session (see `job_wake.rs`). A surface claims
    /// [`JobWake::claim_idle_prompt`](crate::job_wake::JobWake::claim_idle_prompt) while idle.
    pub fn job_wake(&self) -> crate::job_wake::JobWake {
        self.job_wake.clone()
    }

    /// Put hook-provided context (a `SessionStart` hook's stdout / `additionalContext`) in front of
    /// the model as a persisted system message. Blank text is ignored.
    pub fn inject_hook_context(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let seq = self.next_seq();
        let _ = self
            .store
            .add_message(&self.id, seq, Role::System, text, None);
        self.transcript.push(Message::system(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(pid: u32, code: i64, tail: &str) -> JobExit {
        JobExit {
            pid,
            command: "npm run dev\nsecond".into(),
            code,
            tail: tail.into(),
        }
    }

    #[test]
    fn notice_names_job_command_code_and_tail() {
        let w = JobWake::default();
        w.push(&exit(42, 1, "boom"));
        let got = w.drain();
        assert_eq!(got.len(), 1);
        assert!(
            got[0].contains("job 42 `npm run dev` exited 1"),
            "{}",
            got[0]
        );
        assert!(got[0].ends_with("boom"));
        assert!(!w.has_pending());
    }

    #[test]
    fn long_tail_is_capped_on_a_char_boundary() {
        let w = JobWake::default();
        w.push(&exit(1, 0, &"é".repeat(2000)));
        assert!(w.drain()[0].len() < 700);
    }

    #[test]
    fn idle_claim_waits_for_the_coalescing_window() {
        let w = JobWake::default();
        w.push(&exit(1, 0, ""));
        assert!(w.claim_idle_prompt().is_none(), "too fresh to claim");
        std::thread::sleep(COALESCE + Duration::from_millis(100));
        w.push(&exit(2, 0, ""));
        assert!(
            w.claim_idle_prompt().is_none(),
            "newest notice resets the window"
        );
        std::thread::sleep(COALESCE + Duration::from_millis(100));
        let prompt = w.claim_idle_prompt().expect("both coalesce");
        assert!(prompt.contains("job 1") && prompt.contains("job 2"));
        assert!(w.claim_idle_prompt().is_none());
    }

    #[test]
    fn automatic_turns_are_budgeted_and_notices_survive_the_limit() {
        let w = JobWake::default();
        {
            let mut st = w.state.lock().unwrap();
            for _ in 0..MAX_AUTO_TURNS {
                st.auto_turns.push_back(Instant::now());
            }
            st.pending
                .push_back((Instant::now() - COALESCE * 2, "[x]".into()));
        }
        assert!(w.claim_idle_prompt().is_none(), "budget spent");
        assert_eq!(
            w.drain(),
            vec!["[x]".to_string()],
            "still deliverable mid-turn"
        );
    }

    #[test]
    fn overflow_drops_oldest_and_says_so() {
        let w = JobWake::default();
        for pid in 0..(MAX_PENDING as u32 + 3) {
            w.push(&exit(pid, 0, ""));
        }
        let got = w.drain();
        assert_eq!(got.len(), MAX_PENDING + 1);
        assert!(got[0].contains("3 earlier"));
    }
}
