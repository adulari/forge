//! What `forge bench tasks` tells you about a graded attempt: the one line that explains a failed
//! `test.sh`, and the per-task table printed at the end of a run.

use super::suite::TaskResult;

/// Longest failure reason kept on a result row.
const MAX_REASON_CHARS: usize = 160;

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn clip(line: &str) -> String {
    let line = line.trim();
    if line.chars().count() <= MAX_REASON_CHARS {
        return line.to_string();
    }
    let cut: String = line.chars().take(MAX_REASON_CHARS - 1).collect();
    format!("{cut}…")
}

/// The line of a failed `test.sh`'s output that says why: the last one containing `FAIL`, else the
/// last error line (`error:`, `Error`, `panicked at`, `Traceback`, `assert`), else the last
/// non-empty line. `None` when the output is empty.
pub(super) fn failure_reason(output: &str) -> Option<String> {
    let lines: Vec<String> = output
        .lines()
        .map(strip_ansi)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let is_fail = |l: &String| l.contains("FAIL");
    let is_error = |l: &String| {
        let lower = l.to_lowercase();
        ["error", "panicked at", "traceback", "assert"]
            .iter()
            .any(|marker| lower.contains(marker))
    };
    lines
        .iter()
        .rev()
        .find(|l| is_fail(l))
        .or_else(|| lines.iter().rev().find(|l| is_error(l)))
        .or_else(|| lines.last())
        .map(|l| clip(l))
}

/// Why an attempt failed, for its result row: what `test.sh` said, led by an agent error when the
/// turn itself failed (the test output is then usually only the symptom).
pub(super) fn compose_reason(
    agent_error: Option<&str>,
    test_reason: Option<String>,
) -> Option<String> {
    match (agent_error, test_reason) {
        (Some(error), Some(test)) => Some(clip(&format!("agent error: {error} | {test}"))),
        (Some(error), None) => Some(clip(&format!("agent error: {error}"))),
        (None, test) => test,
    }
}

/// The line printed under a task as soon as it is graded.
pub(super) fn verdict_line(r: &TaskResult) -> String {
    let status = match (r.test_exit, r.test_timed_out) {
        (_, true) => "timed out".to_string(),
        (Some(code), _) => format!("exited {code}"),
        (None, _) => "did not run".to_string(),
    };
    let mut line = format!("  {} test.sh {status}", if r.solved { "✓" } else { "✗" });
    if let Some(secs) = r.test_secs {
        line.push_str(&format!(" in {secs:.1}s"));
    }
    if let Some(secs) = r.agent_secs {
        line.push_str(&format!(" · agent {secs:.1}s"));
    }
    if let (false, Some(reason)) = (r.solved, &r.reason) {
        line.push_str(&format!(": {reason}"));
    }
    line
}

fn seconds(secs: Option<f64>) -> String {
    secs.map_or_else(|| "-".to_string(), |s| format!("{s:.1}s"))
}

/// One line per attempt: verdict, agent and test time, and why a failed attempt failed.
pub(super) fn summary_table(results: &[TaskResult]) -> String {
    let width = results
        .iter()
        .map(|r| r.task.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = format!(
        "{:<width$}  {:<6}  {:>8}  {:>7}  note\n",
        "task", "result", "agent", "test"
    );
    for r in results {
        let verdict = if r.solved { "PASS" } else { "FAIL" };
        out.push_str(&format!(
            "{:<width$}  {:<6}  {:>8}  {:>7}  {}\n",
            r.task,
            verdict,
            seconds(r.agent_secs),
            seconds(r.test_secs),
            r.reason.as_deref().unwrap_or("")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_fail_line_wins_over_later_noise() {
        let out = "running checks\nFAIL: first thing\nok: second\nFAIL: total is 2, want 3\nbye\n";
        assert_eq!(
            failure_reason(out).as_deref(),
            Some("FAIL: total is 2, want 3")
        );
    }

    #[test]
    fn without_a_fail_line_the_last_error_line_is_used() {
        let out = "collecting\nTraceback (most recent call last):\n  File x\nAssertionError: 2 != 3\nSummary: done\n";
        assert_eq!(
            failure_reason(out).as_deref(),
            Some("AssertionError: 2 != 3")
        );
        let rust = "test result: FAILED. 0 passed\n";
        assert_eq!(
            failure_reason(rust).as_deref(),
            Some("test result: FAILED. 0 passed")
        );
    }

    #[test]
    fn plain_output_falls_back_to_its_last_line_and_empty_output_to_none() {
        assert_eq!(
            failure_reason("a\n\nlast line\n\n").as_deref(),
            Some("last line")
        );
        assert_eq!(failure_reason("  \n\n"), None);
        assert_eq!(failure_reason(""), None);
    }

    #[test]
    fn colour_codes_are_removed_and_long_lines_are_clipped() {
        let coloured = "\u{1b}[31mFAIL: red\u{1b}[0m";
        assert_eq!(failure_reason(coloured).as_deref(), Some("FAIL: red"));
        let long = format!("FAIL: {}", "x".repeat(400));
        let reason = failure_reason(&long).unwrap();
        assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
        assert!(reason.ends_with('…'));
    }

    #[test]
    fn an_agent_error_leads_the_reason_and_the_verdict_line_shows_times() {
        assert_eq!(
            compose_reason(Some("turn failed"), Some("FAIL: x".into())).as_deref(),
            Some("agent error: turn failed | FAIL: x")
        );
        assert_eq!(
            compose_reason(None, Some("FAIL: x".into())).as_deref(),
            Some("FAIL: x")
        );
        assert_eq!(compose_reason(None, None), None);
        let result = TaskResult {
            task: "t".into(),
            attempt: 1,
            solved: false,
            setup_exit: Some(0),
            test_exit: Some(1),
            test_timed_out: false,
            session_id: None,
            agent_secs: Some(12.34),
            test_secs: Some(0.46),
            reason: Some("FAIL: nope".into()),
        };
        assert_eq!(
            verdict_line(&result),
            "  ✗ test.sh exited 1 in 0.5s · agent 12.3s: FAIL: nope"
        );
        let solved = TaskResult {
            solved: true,
            test_exit: Some(0),
            reason: None,
            ..result
        };
        assert_eq!(
            verdict_line(&solved),
            "  ✓ test.sh exited 0 in 0.5s · agent 12.3s"
        );
    }

    #[test]
    fn the_table_lists_every_attempt_with_times_and_the_reason() {
        let row = |task: &str, solved, agent, test, reason: Option<&str>| TaskResult {
            task: task.into(),
            attempt: 1,
            solved,
            setup_exit: None,
            test_exit: Some(i32::from(!solved)),
            test_timed_out: false,
            session_id: None,
            agent_secs: agent,
            test_secs: test,
            reason: reason.map(str::to_string),
        };
        let table = summary_table(&[
            row("fix", false, Some(12.34), Some(0.5), Some("FAIL: nope")),
            row("longer-name", true, Some(3.0), Some(0.1), None),
            row(
                "skipped-agent",
                false,
                None,
                None,
                Some("setup.sh exited 2"),
            ),
        ]);
        let lines: Vec<&str> = table.lines().collect();
        assert!(lines[0].starts_with("task"), "{table}");
        assert!(
            lines[1].contains("FAIL") && lines[1].contains("12.3s"),
            "{table}"
        );
        assert!(lines[1].ends_with("FAIL: nope"), "{table}");
        assert!(
            lines[2].contains("PASS") && lines[2].trim_end().ends_with("0.1s"),
            "{table}"
        );
        assert!(
            lines[3].contains("setup.sh exited 2") && lines[3].contains('-'),
            "{table}"
        );
    }
}
