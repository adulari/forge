//! Pass/fail read from a test run's own output, for a command whose exit status a pipe masks
//! (`pytest -q 2>&1 | tail -30` exits with `tail`'s status, so the status proves nothing).

use crate::completion::{
    has_standalone_background_operator, strip_heredoc_bodies, strip_quoted_literals,
    verification_family, VerificationFamily,
};
use regex::Regex;
use std::sync::LazyLock;

/// One test runner's summary lines: `(pass, fail)`, both multi-line patterns. A failure marker
/// anywhere wins over a pass marker, so a mixed run is never read as green.
const RUNNERS: &[(&str, &str)] = &[
    // pytest, jest and vitest (`5 passed`, `1 failed`, `1 error`) and pytest's `FAILED path::test`.
    (
        r"\b[1-9]\d* passed\b",
        r"\b[1-9]\d* (?:failed|errors?)\b|^FAILED\b",
    ),
    (
        r"(?m)^test result: ok\.",
        r"test result: FAILED|^test .* \.\.\. FAILED$|^error: test failed|^error: \d+ targets? failed|^error: could not compile|^error\[E\d+\]",
    ),
    // unittest: a bare `OK` after `Ran N tests`; its failure line is the `FAILED (...)` caught above.
    (r"^OK(?: \(.*\))?\s*$", r"^FAILED \((?:failures|errors)="),
    // node --test (TAP `# fail 0`, spec reporter `ℹ fail 0`).
    (r"^(?:#|ℹ) fail 0\b", r"^(?:#|ℹ) fail [1-9]"),
    (
        r"^(?:ok\s+\S+\s+(?:\(cached\)|[\d.]+s)|PASS)\s*$",
        r"^FAIL\b|^--- FAIL",
    ),
];

static COMPILED: LazyLock<Vec<(Regex, Regex)>> = LazyLock::new(|| {
    RUNNERS
        .iter()
        .map(|(pass, fail)| {
            (
                Regex::new(&format!("(?m){pass}")).expect("pass pattern is valid"),
                Regex::new(&format!("(?m){fail}")).expect("fail pattern is valid"),
            )
        })
        .collect()
});

/// `Some(true)` when the output carries an unambiguous pass, `Some(false)` when it shows a
/// failure, `None` when it says nothing either way.
pub(crate) fn output_verdict(output: &str) -> Option<bool> {
    let mut passed = false;
    for (pass, fail) in COMPILED.iter() {
        if fail.is_match(output) {
            return Some(false);
        }
        passed |= pass.is_match(output);
    }
    passed.then_some(true)
}

/// A single-line command whose only masking is a pipe and whose first stage runs tests: the
/// output of the pipe (`2>&1 | tail -30`) can still carry the run's verdict.
pub(crate) fn only_a_pipe_masks_status(command: &str) -> bool {
    let shell = strip_heredoc_bodies(&command.to_ascii_lowercase());
    let operators = strip_quoted_literals(&shell);
    let bytes = operators.as_bytes();
    let first_pipe = bytes.iter().enumerate().position(|(index, byte)| {
        *byte == b'|'
            && index.checked_sub(1).and_then(|i| bytes.get(i)) != Some(&b'|')
            && bytes.get(index + 1) != Some(&b'|')
    });
    let Some(pipe) = first_pipe else {
        return false;
    };
    let first_stage = &shell[..pipe];
    shell.lines().filter(|line| !line.trim().is_empty()).count() == 1
        && !operators.contains(';')
        && !operators.contains("||")
        && !has_standalone_background_operator(&operators)
        // `cat tests/x.log | tail` mentions tests without running any.
        && !first_stage.split_whitespace().next().is_some_and(|word| {
            matches!(word, "echo" | "printf" | "cat" | "head" | "tail" | "grep" | "rg" | "sed" | "awk" | "ls" | "find")
        })
        && verification_family(first_stage) == Some(VerificationFamily::Test)
}

#[cfg(test)]
mod tests {
    use super::output_verdict;

    #[test]
    fn each_runner_reads_both_ways() {
        for (output, verdict) in [
            ("..........\n12 passed in 0.31s", true),
            ("1 failed, 11 passed in 0.31s", false),
            ("FAILED tests/test_a.py::test_x - assert 1 == 2", false),
            ("ERROR collecting tests\n1 error in 0.10s", false),
            ("Tests:       5 passed, 5 total", true),
            ("Tests:       1 failed, 4 passed, 5 total", false),
            ("test result: ok. 9 passed; 0 failed; 0 ignored", true),
            ("test result: FAILED. 8 passed; 1 failed; 0 ignored", false),
            ("error[E0308]: mismatched types", false),
            ("error: could not compile `forge-core` (lib test)", false),
            ("Ran 4 tests in 0.002s\n\nOK", true),
            ("Ran 4 tests in 0.002s\n\nOK (skipped=1)", true),
            ("Ran 4 tests in 0.002s\n\nFAILED (failures=1)", false),
            ("# pass 5\n# fail 0", true),
            ("# pass 4\n# fail 1", false),
            ("ok  \texample.com/pkg\t0.012s", true),
            ("ok  \texample.com/pkg\t(cached)", true),
            ("PASS", true),
            (
                "--- FAIL: TestX (0.00s)\nFAIL\nFAIL\texample.com/pkg\t0.01s",
                false,
            ),
        ] {
            assert_eq!(output_verdict(output), Some(verdict), "{output}");
        }
    }

    #[test]
    fn output_without_a_verdict_is_not_one() {
        for output in [
            "",
            "collected 0 items\n\nno tests ran in 0.01s",
            "Compiling forge-core v2.23.0",
            "0 failed",
            "some tests passed validation",
            "ok",
        ] {
            assert_eq!(output_verdict(output), None, "{output}");
        }
    }
}
