//! What one executed tool call looked like, reduced to what the progress ledger needs.
//!
//! Text similarity alone cannot tell a stuck model from a busy one: seven consecutive successful
//! edit/test steps that all open `Now let me update the …` repeat themselves exactly as much as
//! thirty steps of `You're right — I looped` that re-read the same file. What separates them is
//! whether the tool calls got the model anywhere, and that is what [`CallObservation`] carries:
//! the call's signature, whether it succeeded, whether it wrote a file, and fingerprints of the
//! lines its result printed. The judgement itself lives in [`crate::progress_ledger`].
//!
//! Built where the result is in hand so the (possibly huge) text is hashed once and never copied.

mod text;
mod waiting;

use std::hash::{Hash, Hasher};
use text::{
    error_text, excerpt, has_body, hash_lines, line_fingerprints, mask_digits, stable_fingerprint,
};

pub(crate) use text::hash_str;

/// Tools that cannot change the working tree: a step made only of these is never a reason to
/// look at it. Everything else (a shell command, a script, an external server's tool) might.
const TREE_SAFE_TOOLS: [&str; 14] = [
    "read_file",
    "list_dir",
    "glob",
    "search",
    "web_fetch",
    "web_search",
    "lattice",
    "shell_job",
    "update_tasks",
    "ask_user",
    "present_plan",
    "spawn_agents",
    "browser_network",
    "proxy_network",
];

/// Tools whose first read of a path is information by itself, whatever the lines in it repeat.
const FIRST_READ_TOOLS: [&str; 2] = ["read_file", "list_dir"];

#[derive(Debug, Clone)]
pub(crate) struct CallObservation {
    signature: u64,
    tool: String,
    label: String,
    ok: bool,
    mutating: bool,
    waiting: bool,
    opaque: bool,
    silent: bool,
    check_flip: bool,
    check: Option<&'static str>,
    result_fp: u64,
    failure_fp: u64,
    error_head: Option<String>,
    cause: Option<u64>,
    lines: Vec<u64>,
    read_targets: Vec<u64>,
}

impl CallObservation {
    pub(crate) fn ok(&self) -> bool {
        self.ok
    }

    /// Whether this call was a file write that succeeded.
    pub(crate) fn wrote(&self) -> bool {
        self.ok && self.mutating
    }

    pub(crate) fn signature(&self) -> u64 {
        self.signature
    }

    pub(crate) fn tool(&self) -> &str {
        &self.tool
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn mutating(&self) -> bool {
        self.mutating
    }

    /// A call that waits for something outside the turn (see [`waiting`]): neither progress nor
    /// a loop.
    pub(crate) fn waiting(&self) -> bool {
        self.waiting
    }

    /// The call could have changed the working tree without being a recognised write: a shell
    /// command, a script, an external tool.
    pub(crate) fn opaque(&self) -> bool {
        self.opaque
    }

    /// A shell command that succeeded and printed nothing: all it says is that it ran, so its
    /// effect, if any, is somewhere else.
    pub(crate) fn silent(&self) -> bool {
        self.silent
    }

    /// A verification family that was failing passed on this call.
    pub(crate) fn check_flip(&self) -> bool {
        self.check_flip
    }

    /// The verification family (`test`, `build`, ...) this call ran, when it was one command of
    /// one family.
    pub(crate) fn check(&self) -> Option<&'static str> {
        self.check
    }

    /// Hash of the result's line fingerprints: identical across runs of the same call that differ
    /// only in the shell's `exit N in Xms` header (output that changes, a poll's progress,
    /// hashes differently). A failure's lines have their digits masked first.
    pub(crate) fn result_fp(&self) -> u64 {
        self.result_fp
    }

    /// What a failing result says without its durations, times and ids: equal for two runs of a
    /// check that fail the same way.
    pub(crate) fn failure_fp(&self) -> u64 {
        self.failure_fp
    }

    /// The line of a failure that says what went wrong; `None` when there was none to read.
    pub(crate) fn error_head(&self) -> Option<&str> {
        self.error_head.as_deref()
    }

    /// The failure's cause, ignoring digits: `attempt 3 refused` and `no such file: a-4.rs` and
    /// `no such file: a-5.rs` are each one.
    pub(crate) fn cause(&self) -> Option<u64> {
        self.cause
    }

    pub(crate) fn lines(&self) -> &[u64] {
        &self.lines
    }

    /// The paths this call read for the first time, when it was a read.
    pub(crate) fn read_targets(&self) -> &[u64] {
        &self.read_targets
    }

    /// Anonymous observation, for fixtures that carry only a signature and a result.
    pub(crate) fn new(signature: u64, ok: bool, mutating: bool, result: &str) -> Self {
        let lines = line_fingerprints(result, !ok);
        let error = if ok { None } else { error_text(result) };
        Self {
            signature,
            tool: "tool".to_string(),
            label: format!("call {:08x}", signature & 0xffff_ffff),
            ok,
            mutating,
            waiting: false,
            opaque: false,
            silent: false,
            check_flip: false,
            check: None,
            result_fp: hash_lines(&lines),
            failure_fp: stable_fingerprint(result),
            cause: error.as_deref().map(|e| hash_str(&mask_digits(e))),
            error_head: error,
            lines,
            read_targets: Vec::new(),
        }
    }

    pub(crate) fn for_call(
        call: &forge_types::ToolCall,
        ok: bool,
        mutating: bool,
        result: &str,
    ) -> Self {
        let mut observation = Self::new(call_signature(call), ok, mutating, result);
        observation.tool = call.name.clone();
        observation.label = call_label(call);
        observation.waiting = waiting::is_waiting(call, ok, result);
        observation.opaque = !mutating && !TREE_SAFE_TOOLS.contains(&call.name.as_str());
        observation.silent = call.name == "shell" && ok && !has_body(result);
        if FIRST_READ_TOOLS.contains(&call.name.as_str()) {
            observation.read_targets = read_targets(call);
        }
        observation
    }

    pub(crate) fn with_check(mut self, family: Option<&'static str>, flipped: bool) -> Self {
        self.check = family;
        self.check_flip = flipped;
        self
    }

    #[cfg(test)]
    pub(crate) fn named(mut self, tool: &str) -> Self {
        self.tool = tool.to_string();
        self
    }

    #[cfg(test)]
    pub(crate) fn waiting_for_job(mut self) -> Self {
        self.waiting = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn flipping_a_check(self) -> Self {
        self.with_check(None, true)
    }

    #[cfg(test)]
    pub(crate) fn running_check(self, family: &'static str) -> Self {
        self.with_check(Some(family), false)
    }

    #[cfg(test)]
    pub(crate) fn opaquely(mut self) -> Self {
        self.opaque = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn silently(mut self) -> Self {
        self.silent = true;
        self.opaque = true;
        self
    }

    #[cfg(test)]
    pub(crate) fn reading(mut self, path: &str) -> Self {
        self.read_targets = vec![hash_str(path)];
        self
    }
}

/// The paths a read names, hashed.
fn read_targets(call: &forge_types::ToolCall) -> Vec<u64> {
    let paths = if call.name == "read_file" {
        crate::recent_files::read_paths(&call.args)
    } else {
        ["path", "dir"]
            .iter()
            .find_map(|key| call.args.get(*key).and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .into_iter()
            .collect()
    };
    paths
        .iter()
        .map(|path| hash_str(&format!("{}:{path}", call.name)))
        .collect()
}

/// Stable hash of one call's tool name and JSON arguments (object keys are ordered, so equal
/// arguments hash equal).
pub(crate) fn call_signature(call: &forge_types::ToolCall) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    call.name.hash(&mut h);
    call.args.to_string().hash(&mut h);
    h.finish()
}

/// `read_file src/lib.rs`, `shell cargo test`: the tool and the one argument that says what it
/// was aimed at, short enough for a one-line report.
pub(crate) fn call_label(call: &forge_types::ToolCall) -> String {
    let target = if call.name == forge_tools::PYTHON_TOOL {
        crate::eval_kernel::cell_code(&call.args).and_then(|code| {
            code.lines()
                .map(str::trim)
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .map(str::to_string)
        })
    } else if call.name == "shell" {
        forge_types::shell_command_text(&call.args).and_then(|text| {
            text.lines()
                .find(|l| !l.trim().is_empty())
                .map(str::to_string)
        })
    } else {
        ["path", "file_path", "pattern", "query", "url", "name"]
            .iter()
            .find_map(|key| call.args.get(*key).and_then(serde_json::Value::as_str))
            .map(str::to_string)
    };
    match target {
        Some(target) => excerpt(&format!("{} {}", call.name, target.trim())),
        None => call.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::text::{line_fingerprints, EXCERPT_CHARS};
    use super::*;

    fn shell(command: &str) -> forge_types::ToolCall {
        forge_types::ToolCall {
            id: "c".into(),
            name: "shell".into(),
            args: serde_json::json!({ "command": command }),
        }
    }

    #[test]
    fn shell_timing_in_the_header_does_not_change_the_fingerprints() {
        let a = line_fingerprints("shell: exit 0 in 42ms\n\nsame output\nmore output", false);
        let b = line_fingerprints("shell: exit 0 in 19ms\n\nsame output\nmore output", false);
        assert_eq!(a, b);
        let a = CallObservation::new(1, true, false, "shell: exit 0 in 42ms\nsame");
        let b = CallObservation::new(1, true, false, "shell: exit 0 in 19ms\nsame");
        assert_eq!(a.result_fp(), b.result_fp());
        let c = CallObservation::new(1, true, false, "shell: exit 0 in 19ms\nother");
        assert_ne!(a.result_fp(), c.result_fp());
        let poll_a = CallObservation::new(1, true, false, "shell: exit 0 in 4ms\n\nbuilt 3 of 40");
        let poll_b = CallObservation::new(1, true, false, "shell: exit 0 in 4ms\n\nbuilt 4 of 40");
        assert_ne!(
            poll_a.result_fp(),
            poll_b.result_fp(),
            "a poll whose counter moves is not an identical result"
        );
    }

    #[test]
    fn a_silent_shell_command_is_the_same_result_whatever_its_timing() {
        assert_eq!(
            line_fingerprints("shell: exit 0 in 3ms", false),
            line_fingerprints("shell: exit 0 in 41ms", false)
        );
    }

    #[test]
    fn varying_only_the_quoted_query_of_a_short_result_is_not_new() {
        assert_eq!(
            line_fingerprints("no matches for 'a'", false),
            line_fingerprints("no matches for 'b'", false)
        );
    }

    #[test]
    fn a_poll_mode_wait_is_recognised_only_for_shell() {
        let result = "shell: not ready after 100s (last exit 1) — call shell again with the same poll args to keep waiting.";
        assert!(CallObservation::for_call(&shell("gh run watch"), true, false, result).waiting());
        let other = forge_types::ToolCall {
            id: "c".into(),
            name: "read_file".into(),
            args: serde_json::json!({ "path": "a" }),
        };
        assert!(!CallObservation::for_call(&other, true, false, result).waiting());
        assert!(!CallObservation::for_call(&shell("ls"), true, false, "shell: exit 0").waiting());
    }

    #[test]
    fn labels_name_the_tool_and_its_target() {
        let read = forge_types::ToolCall {
            id: "c".into(),
            name: "read_file".into(),
            args: serde_json::json!({ "path": "src/lib.rs" }),
        };
        assert_eq!(call_label(&read), "read_file src/lib.rs");
        assert_eq!(call_label(&shell("cargo test\nsecond")), "shell cargo test");
        let long = shell(&"x".repeat(200));
        assert!(call_label(&long).chars().count() <= EXCERPT_CHARS + 1);
    }

    #[test]
    fn a_failed_shell_result_reports_its_cause_not_the_exit_header() {
        let obs = CallObservation::for_call(
            &shell("nope"),
            false,
            false,
            "shell: exit 127 in 3ms\n\nsh: 1: nope: not found",
        );
        assert_eq!(obs.error_head(), Some("sh: 1: nope: not found"));
        assert!(CallObservation::new(1, true, false, "fine")
            .error_head()
            .is_none());
        let silent =
            CallObservation::for_call(&shell("grep nope f"), false, false, "shell: exit 1 in 3ms");
        assert_eq!(
            silent.error_head(),
            None,
            "a failure with no words has no cause to compare"
        );
    }

    fn result_of(name: &str, command: &str, ok: bool, result: &str) -> CallObservation {
        let call = forge_types::ToolCall {
            id: "c".into(),
            name: name.into(),
            args: if name == "shell_job" {
                serde_json::json!({ "action": command, "id": 4242 })
            } else {
                serde_json::json!({ "command": command })
            },
        };
        CallObservation::for_call(&call, ok, false, result)
    }

    #[test]
    fn calls_that_wait_for_something_outside_the_turn_are_recognised() {
        let running = "job 4242 is running\n  command: cargo build\n  log: /tmp/j.log";
        assert!(result_of("shell_job", "status", true, running).waiting());
        let log = "job 4242 is running — last 50 lines of /tmp/j.log:\n\ncompiling";
        assert!(result_of("shell_job", "log", true, log).waiting());
        assert!(!result_of(
            "shell_job",
            "status",
            true,
            "job 4242 is exited 0\n  command: x"
        )
        .waiting());
        let cut = "shell: timed out after 120s (killed) in 120013ms\n\nwatching run 9";
        assert!(result_of("shell", "gh run watch 9", false, cut).waiting());
        let ok = "shell: exit 0 in 60004ms\n\nin progress";
        assert!(result_of("shell", "sleep 60; gh run view 9", true, ok).waiting());
        assert!(result_of("shell", "gh run watch 9", true, ok).waiting());
        assert!(!result_of(
            "shell",
            "sleep 60; false",
            false,
            "shell: exit 1 in 60004ms"
        )
        .waiting());
        assert!(!result_of("shell", "cargo test", true, "shell: exit 0 in 4210ms").waiting());
        assert!(!result_of("shell", "tail -n 1 build.log", true, ok).waiting());
    }

    #[test]
    fn a_failure_is_the_same_failure_whatever_its_timing_and_not_when_it_moved() {
        let run = |extra: &str| {
            let result = format!(
                "shell: exit 1 in 953ms\n\nFAILED tests::header\nassertion failed: left == right\n{extra}"
            );
            CallObservation::new(1, false, false, &result).failure_fp()
        };
        assert_eq!(
            run("finished in 0.41s\nstarted 2026-10-23 12:00:01"),
            run("finished in 0.52s\nstarted 2026-10-23 12:07:45")
        );
        assert_ne!(run(""), run("FAILED tests::footer"));
    }

    #[test]
    fn only_a_call_that_may_change_files_unseen_is_opaque() {
        let named = |name: &str| {
            let call = forge_types::ToolCall {
                id: "c".into(),
                name: name.into(),
                args: serde_json::json!({ "path": "a" }),
            };
            CallObservation::for_call(&call, true, false, "x")
        };
        for pure in [
            "read_file",
            "search",
            "glob",
            "list_dir",
            "web_fetch",
            "shell_job",
        ] {
            assert!(!named(pure).opaque(), "{pure}");
        }
        for open in ["shell", "run_code", "mcp__server__tool"] {
            assert!(named(open).opaque(), "{open}");
        }
        let call = forge_types::ToolCall {
            id: "c".into(),
            name: "write_file".into(),
            args: serde_json::json!({ "path": "a" }),
        };
        assert!(
            !CallObservation::for_call(&call, true, true, "wrote").opaque(),
            "a recognised write is not an unseen change"
        );
    }

    #[test]
    fn a_silent_shell_command_has_nothing_to_say_and_a_read_names_its_paths() {
        assert!(result_of("shell", "mkdir out", true, "shell: exit 0 in 3ms").silent());
        assert!(!result_of("shell", "ls", true, "shell: exit 0 in 3ms\n\na\nb").silent());
        assert!(!result_of("shell", "false", false, "shell: exit 1 in 3ms").silent());
        let call = forge_types::ToolCall {
            id: "c".into(),
            name: "read_file".into(),
            args: serde_json::json!({ "paths": ["a.rs:10-20", "b.rs"] }),
        };
        let read = CallObservation::for_call(&call, true, false, "x");
        assert_eq!(read.read_targets().len(), 2);
    }

    #[test]
    fn noise_is_masked_but_the_numbers_that_tell_things_apart_are_not() {
        use super::text::mask_noise;
        assert_eq!(
            mask_noise("attempt 3 of 20: connection refused (os error 111)"),
            mask_noise("attempt 4 of 20: connection refused (os error 111)")
        );
        assert_eq!(
            mask_noise("finished in 0.41s at 12:00:01 on 2026-10-23"),
            mask_noise("finished in 3.50s at 23:59:59 on 2026-10-24")
        );
        assert_eq!(
            mask_noise("pid 4242 exited, run 77"),
            mask_noise("pid 9 exited, run 3")
        );
        assert_ne!(
            mask_noise("suite 3 case 5: fixture_2 is missing"),
            mask_noise("suite 4 case 1: fixture_2 is missing")
        );
        assert_ne!(
            mask_noise("left: 1\nright: 2"),
            mask_noise("left: 1\nright: 3")
        );
    }
}
