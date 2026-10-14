//! Silence guard: keep the user informed during long tool-only stretches.
//!
//! A model can run hundreds of distinct, successful tool calls (mostly `shell`) without a single
//! word of text. None of the loop guards fire on that: the doom loop needs repeated arguments,
//! the failure loop repeated failures, stagnation unseen output, and the narration stall repeated
//! wording (it skips empty text). The user saw nothing readable for half an hour. This guard
//! counts tool steps since the last user-visible text and, every [`SILENCE_NUDGE_STEPS`] such
//! steps (or after [`SILENCE_NUDGE_SECS`]), asks the model for a one-line progress note and shows
//! a harness-written summary of the stretch, so the user sees progress even if the model ignores
//! the ask. It never halts: a silent model can still be productive, and halting stays with the
//! guards that detect real loops.

use super::*;

/// Silent tool steps that trigger a progress ask.
pub(crate) const SILENCE_NUDGE_STEPS: usize = 12;
/// Wall-clock silence that triggers a progress ask once at least [`SILENCE_MIN_TIMED_STEPS`] ran.
pub(crate) const SILENCE_NUDGE_SECS: u64 = 180;
const SILENCE_MIN_TIMED_STEPS: usize = 4;
/// Steps without a successful file write after which the ask also requests a findings summary.
pub(crate) const EXPLORATION_STEPS: usize = 40;
const RECENT_COMMANDS: usize = 3;
const COMMAND_HEAD_CHARS: usize = 60;

/// What one firing delivers: a system message for the model and a progress line for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SilenceNudge {
    pub(crate) model_hint: String,
    pub(crate) progress_line: String,
}

#[derive(Debug)]
pub(crate) struct SilenceTracker {
    silent_steps: usize,
    since: std::time::Instant,
    tool_counts: std::collections::BTreeMap<String, usize>,
    recent_commands: std::collections::VecDeque<String>,
    steps_without_write: usize,
    exploration_flagged: bool,
}

impl SilenceTracker {
    pub(crate) fn new(now: std::time::Instant) -> Self {
        Self {
            silent_steps: 0,
            since: now,
            tool_counts: Default::default(),
            recent_commands: Default::default(),
            steps_without_write: 0,
            exploration_flagged: false,
        }
    }

    fn reset_stretch(&mut self, now: std::time::Instant) {
        self.silent_steps = 0;
        self.since = now;
        self.tool_counts.clear();
        self.recent_commands.clear();
    }

    /// Record one tool step. `visible_text` is whether the step's response showed the user any
    /// text; `wrote` whether a file write in it succeeded.
    pub(crate) fn observe(
        &mut self,
        visible_text: bool,
        calls: &[forge_types::ToolCall],
        wrote: bool,
        now: std::time::Instant,
    ) -> Option<SilenceNudge> {
        if wrote {
            self.steps_without_write = 0;
            self.exploration_flagged = false;
        } else if !calls.is_empty() {
            self.steps_without_write += 1;
        }
        if visible_text {
            self.reset_stretch(now);
            return None;
        }
        if calls.is_empty() {
            return None;
        }
        self.silent_steps += 1;
        for call in calls {
            *self.tool_counts.entry(call.name.clone()).or_default() += 1;
            if let Some(head) = command_head(call) {
                self.recent_commands.push_back(head);
                if self.recent_commands.len() > RECENT_COMMANDS {
                    self.recent_commands.pop_front();
                }
            }
        }
        let timed_out = now.duration_since(self.since).as_secs() >= SILENCE_NUDGE_SECS
            && self.silent_steps >= SILENCE_MIN_TIMED_STEPS;
        if self.silent_steps < SILENCE_NUDGE_STEPS && !timed_out {
            return None;
        }
        let nudge = self.build_nudge();
        self.reset_stretch(now);
        Some(nudge)
    }

    fn build_nudge(&mut self) -> SilenceNudge {
        let calls: usize = self.tool_counts.values().sum();
        let mut model_hint = format!(
            "You have made {calls} tool calls without a word to the user. In your next response, \
             write one or two sentences on what you have found and what you are doing next, and \
             continue with tool calls in that same response."
        );
        if self.steps_without_write >= EXPLORATION_STEPS && !self.exploration_flagged {
            self.exploration_flagged = true;
            model_hint.push_str(&format!(
                " You have also gone {} steps without changing a file: summarize your findings so \
                 far and state the decision you are working toward.",
                self.steps_without_write
            ));
        }
        let mut by_tool: Vec<(&String, &usize)> = self.tool_counts.iter().collect();
        by_tool.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let tools = by_tool
            .iter()
            .map(|(name, n)| format!("{n} {name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut progress_line =
            format!("{} steps since the last update: {tools}", self.silent_steps);
        if !self.recent_commands.is_empty() {
            let latest: Vec<&str> = self.recent_commands.iter().map(String::as_str).collect();
            progress_line.push_str(&format!("; latest: {}", latest.join(" · ")));
        }
        SilenceNudge {
            model_hint,
            progress_line,
        }
    }
}

/// The first line of a shell call's command (or of its first batched command), shortened.
fn command_head(call: &forge_types::ToolCall) -> Option<String> {
    if call.name != "shell" {
        return None;
    }
    let text = call
        .args
        .get("command")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            call.args
                .get("commands")
                .and_then(serde_json::Value::as_array)
                .and_then(|list| list.first())
                .and_then(serde_json::Value::as_str)
        })?;
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    if line.chars().count() <= COMMAND_HEAD_CHARS {
        return Some(line.to_string());
    }
    let head: String = line.chars().take(COMMAND_HEAD_CHARS).collect();
    Some(format!("{head}…"))
}

impl Session {
    /// Feed one finished tool step to the tracker and deliver a nudge when it fires. Text counts
    /// as visible only when it reached the user (`text_shown` is false while a verification pass
    /// suppresses assistant text).
    pub(crate) fn observe_silence(
        &mut self,
        tracker: &mut SilenceTracker,
        text_shown: bool,
        resp: &forge_provider::ModelResponse,
        observations: &[crate::loop_progress::CallObservation],
    ) -> Result<(), CoreError> {
        let visible_text = text_shown && !resp.content.trim().is_empty();
        let wrote = observations
            .iter()
            .any(crate::loop_progress::CallObservation::wrote);
        let calls: Vec<forge_types::ToolCall> = resp
            .tool_calls
            .iter()
            .map(|call| self.resolve_deferred_call(call))
            .collect();
        match tracker.observe(visible_text, &calls, wrote, std::time::Instant::now()) {
            Some(nudge) => self.deliver_silence_nudge(nudge),
            None => Ok(()),
        }
    }

    /// Deliver a silence nudge: the progress line to the user, the ask to the model as a system
    /// message after this step's tool results. A `minimal` harness profile keeps only the line.
    pub(crate) fn deliver_silence_nudge(&mut self, nudge: SilenceNudge) -> Result<(), CoreError> {
        self.presenter
            .emit(PresenterEvent::Progress(nudge.progress_line));
        if !self.advisory_nudges_on() {
            return Ok(());
        }
        let seq = self.next_seq();
        self.store
            .add_message(&self.id, seq, Role::System, &nudge.model_hint, None)?;
        self.transcript.push(Message::system(nudge.model_hint));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(cmd: &str) -> forge_types::ToolCall {
        forge_types::ToolCall {
            id: "c".into(),
            name: "shell".into(),
            args: serde_json::json!({ "command": cmd }),
        }
    }

    #[test]
    fn fires_after_twelve_silent_steps_and_rearms() {
        let t0 = std::time::Instant::now();
        let mut tracker = SilenceTracker::new(t0);
        for i in 0..SILENCE_NUDGE_STEPS - 1 {
            assert!(tracker
                .observe(false, &[shell(&format!("ls {i}"))], false, t0)
                .is_none());
        }
        let nudge = tracker
            .observe(false, &[shell("cargo test -p forge-agent-core")], false, t0)
            .expect("the twelfth silent step asks for an update");
        assert!(nudge.model_hint.contains("12 tool calls"));
        assert!(nudge
            .progress_line
            .starts_with("12 steps since the last update: 12 shell"));
        assert!(nudge
            .progress_line
            .contains("cargo test -p forge-agent-core"));
        for i in 0..SILENCE_NUDGE_STEPS - 1 {
            assert!(tracker
                .observe(false, &[shell(&format!("ls {i}"))], false, t0)
                .is_none());
        }
        assert!(tracker.observe(false, &[shell("ls")], false, t0).is_some());
    }

    #[test]
    fn visible_text_resets_the_stretch() {
        let t0 = std::time::Instant::now();
        let mut tracker = SilenceTracker::new(t0);
        for _ in 0..SILENCE_NUDGE_STEPS - 1 {
            tracker.observe(false, &[shell("ls")], false, t0);
        }
        assert!(tracker.observe(true, &[shell("ls")], false, t0).is_none());
        for _ in 0..SILENCE_NUDGE_STEPS - 1 {
            assert!(tracker.observe(false, &[shell("ls")], false, t0).is_none());
        }
    }

    #[test]
    fn long_silence_fires_on_time_after_a_few_steps() {
        let t0 = std::time::Instant::now();
        let later = t0 + std::time::Duration::from_secs(SILENCE_NUDGE_SECS);
        let mut tracker = SilenceTracker::new(t0);
        for _ in 0..SILENCE_MIN_TIMED_STEPS - 1 {
            assert!(tracker
                .observe(false, &[shell("sleep 60")], false, later)
                .is_none());
        }
        assert!(tracker
            .observe(false, &[shell("sleep 60")], false, later)
            .is_some());
    }

    #[test]
    fn a_long_stretch_without_writes_also_asks_for_findings_once() {
        let t0 = std::time::Instant::now();
        let mut tracker = SilenceTracker::new(t0);
        let mut asks = Vec::new();
        for _ in 0..EXPLORATION_STEPS + SILENCE_NUDGE_STEPS {
            if let Some(n) = tracker.observe(false, &[shell("grep x")], false, t0) {
                asks.push(n.model_hint);
            }
        }
        let with_findings = asks
            .iter()
            .filter(|h| h.contains("summarize your findings"))
            .count();
        assert_eq!(with_findings, 1, "{asks:?}");
    }
}
