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

use std::hash::{Hash, Hasher};

/// One-line results at most this long are treated as status messages (`no matches for 'x'`,
/// `ok`): their quoted parts are masked so varying only the query does not read as new.
const SHORT_RESULT_CHARS: usize = 160;
/// Longest label or error excerpt kept for a progress report.
const EXCERPT_CHARS: usize = 60;

#[derive(Debug, Clone)]
pub(crate) struct CallObservation {
    signature: u64,
    tool: String,
    label: String,
    ok: bool,
    mutating: bool,
    waiting: bool,
    check_flip: bool,
    result_fp: u64,
    error_head: Option<String>,
    lines: Vec<u64>,
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

    /// A poll-mode wait that came back "not ready yet": the model is waiting on something
    /// external exactly as it was told to, which is neither progress nor a loop.
    pub(crate) fn waiting(&self) -> bool {
        self.waiting
    }

    /// A verification family that was failing passed on this call.
    pub(crate) fn check_flip(&self) -> bool {
        self.check_flip
    }

    /// Hash of the result's line fingerprints: identical across runs of the same call that differ
    /// only in the shell's `exit N in Xms` header (output that changes, a poll's progress, hashes
    /// differently).
    pub(crate) fn result_fp(&self) -> u64 {
        self.result_fp
    }

    pub(crate) fn error_head(&self) -> Option<&str> {
        self.error_head.as_deref()
    }

    pub(crate) fn lines(&self) -> &[u64] {
        &self.lines
    }

    /// Anonymous observation, for fixtures that carry only a signature and a result.
    pub(crate) fn new(signature: u64, ok: bool, mutating: bool, result: &str) -> Self {
        let lines = line_fingerprints(result);
        Self {
            signature,
            tool: "tool".to_string(),
            label: format!("call {:08x}", signature & 0xffff_ffff),
            ok,
            mutating,
            waiting: false,
            check_flip: false,
            result_fp: hash_lines(&lines),
            error_head: (!ok).then(|| error_head(result)),
            lines,
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
        observation.waiting = call.name == "shell" && is_waiting_result(result);
        observation
    }

    pub(crate) fn with_check_flip(mut self, flipped: bool) -> Self {
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
        self.with_check_flip(true)
    }
}

/// Stable hash of one call's tool name and JSON arguments (object keys are ordered, so equal
/// arguments hash equal).
pub(crate) fn call_signature(call: &forge_types::ToolCall) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    call.name.hash(&mut h);
    call.args.to_string().hash(&mut h);
    h.finish()
}

/// Whether a `shell` result is the resumable "call again to keep waiting" answer of poll mode.
pub(crate) fn is_waiting_result(result: &str) -> bool {
    result.trim_start().starts_with("shell: not ready after")
}

/// `read_file src/lib.rs`, `shell cargo test`: the tool and the one argument that says what it
/// was aimed at, short enough for a one-line report.
pub(crate) fn call_label(call: &forge_types::ToolCall) -> String {
    let target = if call.name == "shell" {
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

fn excerpt(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= EXCERPT_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(EXCERPT_CHARS).collect();
    format!("{head}…")
}

/// The line of a failed result that says what went wrong: the first one, unless that is only the
/// shell's `exit N in Xms` header.
fn error_head(result: &str) -> String {
    let mut lines = result.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or("");
    let line = if first.starts_with("shell: exit") {
        lines.next().unwrap_or(first)
    } else {
        first
    };
    excerpt(line)
}

fn hash_lines(lines: &[u64]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn line_fingerprints(result: &str) -> Vec<u64> {
    let mut lines = result.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next();
    let rest: Vec<&str> = lines.collect();
    let mut out = Vec::with_capacity(rest.len() + 1);
    let single_short = rest.is_empty() && first.is_some_and(|l| l.len() <= SHORT_RESULT_CHARS);
    if let Some(first) = first {
        // `shell: exit 0 in 42ms` — the timing differs on every run of an identical command, so a
        // silent command (`true`, `git add`) whose whole result is that header would otherwise
        // look like new output every time it ran.
        let header = if first.starts_with("shell:") {
            mask_digits(first)
        } else {
            first.to_string()
        };
        out.push(hash_str(&if single_short {
            mask_quoted(&header)
        } else {
            header
        }));
    }
    out.extend(rest.into_iter().map(hash_str));
    out
}

/// Every run of digits becomes one `#`: `3ms` and `41ms` are the same timing.
fn mask_digits(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_digits = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_digits {
                out.push('#');
            }
            in_digits = true;
        } else {
            in_digits = false;
            out.push(c);
        }
    }
    out
}

pub(crate) fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Replace everything between matching quote characters with nothing.
fn mask_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut open: Option<char> = None;
    for c in s.chars() {
        match open {
            Some(q) if c == q => {
                open = None;
                out.push(c);
            }
            Some(_) => {}
            None => {
                if matches!(c, '\'' | '"' | '`') {
                    open = Some(c);
                }
                out.push(c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
        let a = line_fingerprints("shell: exit 0 in 42ms\n\nsame output\nmore output");
        let b = line_fingerprints("shell: exit 0 in 19ms\n\nsame output\nmore output");
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
            line_fingerprints("shell: exit 0 in 3ms"),
            line_fingerprints("shell: exit 0 in 41ms")
        );
    }

    #[test]
    fn varying_only_the_quoted_query_of_a_short_result_is_not_new() {
        assert_eq!(
            line_fingerprints("no matches for 'a'"),
            line_fingerprints("no matches for 'b'")
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
    }
}
