//! How a direct-path model learns to use `run_code`: a static system block sent when the tool is
//! advertised, and a one-time per-turn hint after a run of single-call exploration steps.
//!
//! Both texts are constants, so the system block keeps the cached prefix byte-stable and the hint
//! is appended after the step's tool results like every other loop nudge.

use forge_types::{Message, Role, ToolCall};

use crate::Session;

/// Sent as its own system message right after the base prompt whenever `run_code` is in the
/// advertised catalog. Kept out of `FORGE_SYSTEM` because CLI bridges share that prompt and never
/// see the tool.
pub(crate) const GUIDANCE: &str = "\
run_code (one step for a planned chain of tool calls):
- Independent reads/searches: request them together as parallel calls; that is already one step.
- A dependent chain you can already plan (edit, then run the tests; search, then read the files it \
finds; run the tests and keep only the failures): do it in ONE run_code call, not one step per call.
- In the script: `await tools.<tool_name>({...same arguments as a normal call...})` returns the \
output as ONE string (glob and search too: newline-separated). End with `return` and return only \
what you need, e.g. `out.split(\"\\n\").slice(-20).join(\"\\n\")`.
Example `code`:
await tools.edit_file({path: 'src/client.ts', old: 'retries = 3', new: 'retries = 5'});
const test = await tools.shell({command: 'npm test 2>&1 | tail -15'});
return test;";

/// Consecutive single-call exploration steps before the one-time hint.
pub(crate) const STREAK: usize = 3;

pub(crate) const HINT: &str = "You have spent several steps on one call each. Plan the next \
few: request independent reads/searches together as parallel calls, and put a dependent chain \
(edit then test, search then read the hits) in ONE run_code call \
(`await tools.<tool_name>({...})`, end with `return`).";

/// Tools whose single-call steps count toward the streak: reads, searches and shell probes.
const EXPLORATION: &[&str] = &[
    "read_file",
    "search",
    "glob",
    "list_dir",
    "grep",
    "shell",
    "lattice",
    "find_symbol",
    "lsp",
];

/// Turn-local steering state: the current streak and whether the hint was already sent.
#[derive(Debug, Default)]
pub(crate) struct Steer {
    streak: usize,
    hinted: bool,
}

impl Steer {
    /// Fold one model step in. Returns true exactly once per turn, on the step that completes a
    /// streak of [`STREAK`] single-call exploration steps.
    pub(crate) fn observe(&mut self, calls: &[ToolCall]) -> bool {
        let exploring = matches!(calls, [call] if EXPLORATION.contains(&call.name.as_str()));
        self.streak = if exploring { self.streak + 1 } else { 0 };
        if self.hinted || self.streak < STREAK {
            return false;
        }
        self.hinted = true;
        true
    }
}

/// Whether the static guidance block belongs in a request with these tool specs.
pub(crate) fn advertised(specs: &[forge_provider::ToolSpec]) -> bool {
    specs.iter().any(|s| s.name == super::RUN_CODE_TOOL)
}

impl Session {
    /// After a direct-path step: queue the one-time run_code hint when the streak completes.
    /// Silent on CLI bridges, under the `minimal` harness profile, and when codemode is off or
    /// outside the task scope.
    pub(crate) fn steer_codemode(&mut self, steer: &mut Steer, model: &str, calls: &[ToolCall]) {
        if forge_provider::is_cli_bridge(model)
            || !self.config.codemode.enabled
            || !self.advisory_nudges_on()
            || self
                .task_scope
                .as_ref()
                .is_some_and(|scope| !scope.permits_tool(super::RUN_CODE_TOOL))
        {
            return;
        }
        if !steer.observe(calls) {
            return;
        }
        let seq = self.next_seq();
        let _ = self
            .store
            .add_message(&self.id, seq, Role::System, HINT, None);
        self.transcript.push(Message::system(HINT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: "c".into(),
            name: name.into(),
            args: serde_json::json!({}),
        }
    }

    #[test]
    fn hint_fires_once_after_a_streak_of_single_exploration_steps() {
        let mut steer = Steer::default();
        assert!(!steer.observe(&[call("read_file")]));
        assert!(!steer.observe(&[call("shell")]));
        assert!(steer.observe(&[call("search")]));
        for _ in 0..5 {
            assert!(!steer.observe(&[call("read_file")]));
        }
    }

    #[test]
    fn batches_edits_and_run_code_break_the_streak() {
        let mut steer = Steer::default();
        steer.observe(&[call("read_file")]);
        steer.observe(&[call("read_file")]);
        assert!(!steer.observe(&[call("read_file"), call("search")]));
        steer.observe(&[call("read_file")]);
        steer.observe(&[call("read_file")]);
        assert!(!steer.observe(&[call("edit_file")]));
        steer.observe(&[call("read_file")]);
        steer.observe(&[call("read_file")]);
        assert!(!steer.observe(&[call(super::super::RUN_CODE_TOOL)]));
    }

    #[test]
    fn guidance_names_the_call_shape() {
        assert!(GUIDANCE.contains("await tools.<tool_name>({"));
        assert!(GUIDANCE.contains("return"));
    }
}
