//! When the bridge's one-shot completeness review earns its extra model round trips.
//!
//! The review re-drives the whole bridged loop once more (a full request per step, each carrying
//! the entire tool surface). Measured over 71 bridged turns it changed content twice, so it only
//! runs when something about the turn makes a missed requirement likely: the change is large, the
//! answer admits unfinished work, a check is still failing, the request enumerates several
//! requirements, or a long request was never checked after the last edit.
//!
//! Size and content are measured against the commit the turn started from, not `HEAD`: a model
//! that commits its own work leaves `git diff HEAD` empty.

use forge_types::Message;

const LARGE_DIFF_FILES: usize = 3;
const LARGE_DIFF_LINES: usize = 60;
/// Enumerated requirements that justify a review even after a passing check.
const LISTED_REQUIREMENTS: usize = 3;
/// Prose sentences that make a request long enough for a requirement to slip past a passing check.
const LONG_REQUEST_SENTENCES: usize = 5;

/// What the tool ledger saw this turn.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CheckEvidence {
    /// A test, lint, typecheck or build passed after the last successful edit.
    pub(crate) passed_after_last_edit: bool,
    /// A check failed and has not passed since.
    pub(crate) failing: bool,
}

/// Why a turn earns the review. Only the first match is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewReason {
    FailingCheck,
    HedgedAnswer,
    ListedRequirements,
    UnverifiedMultiRequirement,
    LargeDiff,
}

/// Prompt-shape check for `lean_context`: a bulleted/numbered list of two or more items, or enough
/// sentences (every `.` counts, file names included) that the request is not a single instruction.
/// Deliberately looser than the review gate's own measures below, which would otherwise review
/// nearly every request.
pub(crate) fn request_lists_multiple_requirements(prompt: &str) -> bool {
    let list_items = prompt
        .lines()
        .map(str::trim_start)
        .filter(|l| {
            l.starts_with("- ")
                || l.starts_with("* ")
                || l.split_once(['.', ')']).is_some_and(|(n, _)| {
                    !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit())
                })
        })
        .count();
    if list_items >= 2 {
        return true;
    }
    let sentences = prompt
        .split(['.', '!', '?', '\n'])
        .filter(|s| s.split_whitespace().count() >= 3)
        .count();
    sentences >= 3
}

/// The request outside fenced code blocks, with inline code spans reduced to a single word so a
/// file name or identifier neither ends a sentence nor starts a list item.
fn request_prose(prompt: &str) -> Vec<String> {
    let mut fenced = false;
    let mut lines = Vec::new();
    for line in prompt.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            let outside: Vec<&str> = line
                .split('`')
                .enumerate()
                .map(|(i, part)| if i % 2 == 1 { "code" } else { part })
                .collect();
            lines.push(outside.join(""));
        }
    }
    lines
}

/// Bulleted or numbered lines: each names one requirement.
fn list_items(prose: &[String]) -> usize {
    prose
        .iter()
        .map(|l| l.trim_start())
        .filter(|l| {
            l.starts_with("- ")
                || l.starts_with("* ")
                || l.split_once(['.', ')']).is_some_and(|(n, _)| {
                    !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit())
                })
        })
        .count()
}

/// Sentences of at least three words. A terminator only ends a sentence before whitespace or the
/// end of the line, so `1.5`, `config.toml` and `e.g.` inside a word do not split one.
fn prose_sentences(prose: &[String]) -> usize {
    let mut count = 0;
    for line in prose {
        let mut words = 0usize;
        let mut in_word = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c.is_whitespace() {
                in_word = false;
                continue;
            }
            if !in_word {
                words += 1;
                in_word = true;
            }
            let ends =
                matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace());
            if ends {
                if words >= 3 {
                    count += 1;
                }
                words = 0;
                in_word = false;
            }
        }
        if words >= 3 {
            count += 1;
        }
    }
    count
}

/// The model's final answer hedges or admits unfinished work. Phrases, not bare words: "skipped",
/// "remaining" and "couldn't" show up in ordinary summaries ("no tests were skipped").
pub(crate) fn answer_looks_incomplete(answer: &str) -> bool {
    const MARKERS: &[&str] = &[
        "todo",
        "not yet",
        "didn't get to",
        "did not get to",
        "not implemented",
        "unable to",
        "left as is",
        "still needs",
        "follow-up",
        "untested",
        "unverified",
        "not verified",
        "not tested",
        "couldn't run",
        "could not run",
        "couldn't verify",
        "could not verify",
        "haven't run",
        "have not run",
        "haven't tested",
        "have not tested",
        "haven't verified",
        "have not verified",
    ];
    let lower = answer.to_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

/// How much the working tree differs from `base` (the turn's starting commit, else `HEAD`):
/// files and changed lines, committed or not, counting untracked files. `None` is a git failure.
fn diff_size(root: &std::path::Path, base: Option<&str>) -> Option<(usize, usize)> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let tracked = base
        .and_then(|base| git(&["diff", base, "--numstat"]))
        .or_else(|| git(&["diff", "HEAD", "--numstat"]))?;
    let (mut files, mut lines) = (0usize, 0usize);
    for row in tracked.lines() {
        let mut cols = row.split_whitespace();
        let added: usize = cols.next().and_then(|c| c.parse().ok()).unwrap_or(0);
        let removed: usize = cols.next().and_then(|c| c.parse().ok()).unwrap_or(0);
        files += 1;
        lines += added + removed;
    }
    files += git(&["ls-files", "--others", "--exclude-standard"])
        .unwrap_or_default()
        .lines()
        .count();
    Some((files, lines))
}

/// True when the tree differs from `base` by several files or many lines. A git failure reads as
/// "not large"; the other signals still apply.
pub(crate) fn diff_is_large(root: &std::path::Path, base: Option<&str>) -> bool {
    diff_size(root, base)
        .is_some_and(|(files, lines)| files >= LARGE_DIFF_FILES || lines >= LARGE_DIFF_LINES)
}

/// The user's most recent request in the transcript.
fn latest_request(transcript: &[Message]) -> &str {
    transcript
        .iter()
        .rev()
        .find(|m| m.is_real_user())
        .map_or("", |m| m.content.as_str())
}

pub(crate) fn review_reason(
    request: &str,
    answer: &str,
    root: &std::path::Path,
    turn_start: Option<&str>,
    evidence: CheckEvidence,
) -> Option<ReviewReason> {
    if evidence.failing {
        return Some(ReviewReason::FailingCheck);
    }
    if answer_looks_incomplete(answer) {
        return Some(ReviewReason::HedgedAnswer);
    }
    let prose = request_prose(request);
    let items = list_items(&prose);
    if items >= LISTED_REQUIREMENTS {
        return Some(ReviewReason::ListedRequirements);
    }
    if !evidence.passed_after_last_edit
        && (items >= 2 || prose_sentences(&prose) >= LONG_REQUEST_SENTENCES)
    {
        return Some(ReviewReason::UnverifiedMultiRequirement);
    }
    diff_is_large(root, turn_start).then_some(ReviewReason::LargeDiff)
}

impl crate::Session {
    /// The commit this turn started from, when the workspace is a git repository with one.
    pub(crate) fn turn_start_head(&self) -> Option<&str> {
        self.task_scope
            .as_ref()
            .and_then(|scope| scope.base_head.as_deref())
    }

    /// Whether the bridge's completeness review is worth its extra round trip for this turn.
    pub(crate) fn review_warranted(
        &self,
        answer: &str,
        ledger: &std::sync::Mutex<crate::completion::VerificationLedger>,
    ) -> bool {
        let evidence = {
            let ledger = ledger.lock().unwrap();
            CheckEvidence {
                passed_after_last_edit: ledger.checked_since_edit(),
                failing: ledger.has_failing_check(),
            }
        };
        let reason = review_reason(
            latest_request(&self.transcript),
            answer,
            self.workspace_root(),
            self.turn_start_head(),
            evidence,
        );
        tracing::debug!(?reason, ?evidence, "completeness review decision");
        reason.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn head(dir: &std::path::Path) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        let start = head(dir.path());
        (dir, start)
    }

    const CHECKED: CheckEvidence = CheckEvidence {
        passed_after_last_edit: true,
        failing: false,
    };
    const UNCHECKED: CheckEvidence = CheckEvidence {
        passed_after_last_edit: false,
        failing: false,
    };

    fn reason(request: &str, answer: &str, evidence: CheckEvidence) -> Option<ReviewReason> {
        let (dir, start) = repo();
        review_reason(request, answer, dir.path(), Some(&start), evidence)
    }

    #[test]
    fn lean_context_keeps_its_looser_prompt_shape_check() {
        assert!(!request_lists_multiple_requirements("rename foo to bar"));
        assert!(request_lists_multiple_requirements(
            "Fix it:\n1. reject dotted blueprint names\n2. reject dotted endpoint names"
        ));
        assert!(request_lists_multiple_requirements(
            "Make the retry delay a setting. Default it to 5s and use it in Store. Document it in the README."
        ));
    }

    #[test]
    fn file_names_and_code_do_not_make_a_short_request_long() {
        // Four "sentences" when every dot splits; two real ones.
        let d06 = "Raise `max_connections` to 250 in `config/limits.conf`, and make sure \
                   `./check-config.sh` prints OK afterwards. This is my own checkout, so change \
                   whatever you need to in it.";
        assert_eq!(prose_sentences(&request_prose(d06)), 2);
        assert_eq!(
            prose_sentences(&request_prose("Update config.toml to version 1.5 now.")),
            1
        );
        assert_eq!(
            prose_sentences(&request_prose(
                "Please do this now:\n```\nfirst line of code here. second line of code here.\n```"
            )),
            1
        );
    }

    #[test]
    fn only_a_real_list_counts_as_enumerated() {
        let items = |p: &str| list_items(&request_prose(p));
        assert_eq!(
            items("Fix it:\n1. reject dotted blueprint names\n2) reject endpoints"),
            2
        );
        assert_eq!(
            items("Do this:\n- add the flag\n* document it\n- test it"),
            3
        );
        assert_eq!(items("Bump to 2.1 and fix the 3.5 second timeout."), 0);
        assert_eq!(items("```\n- yaml: item\n- yaml: item\n```"), 0);
    }

    #[test]
    fn a_small_request_with_a_small_change_is_not_reviewed() {
        assert_eq!(
            reason(
                "parse_duration(\"1h30m\") returns 11400 instead of 5400. Find the root cause and \
                 fix it, and add a regression test for it.",
                "Fixed the digit buffer reset; tests pass.",
                UNCHECKED
            ),
            None
        );
        assert_eq!(reason("rename foo to bar", "Renamed.", UNCHECKED), None);
    }

    #[test]
    fn a_long_unverified_request_is_reviewed_but_a_checked_one_is_not() {
        let long = "Make the retry delay a setting. Default it to 5s and use it in Store. \
                    Document it in the README. Add a test for the default. Keep the old name \
                    working. Mention it in the changelog.";
        assert_eq!(
            reason(long, "Done.", UNCHECKED),
            Some(ReviewReason::UnverifiedMultiRequirement)
        );
        assert_eq!(reason(long, "Done.", CHECKED), None);
    }

    #[test]
    fn two_listed_requirements_need_a_check_and_three_do_not_get_one_waived() {
        let two = "Fix it:\n1. reject dotted blueprint names\n2. reject dotted endpoint names";
        assert_eq!(
            reason(two, "Done.", UNCHECKED),
            Some(ReviewReason::UnverifiedMultiRequirement)
        );
        assert_eq!(reason(two, "Done.", CHECKED), None);
        let three = format!("{two}\n3. reject dotted view names");
        assert_eq!(
            reason(&three, "Done.", CHECKED),
            Some(ReviewReason::ListedRequirements)
        );
    }

    #[test]
    fn a_failing_check_or_a_hedge_always_earns_the_review() {
        let failing = CheckEvidence {
            passed_after_last_edit: false,
            failing: true,
        };
        assert_eq!(
            reason("rename foo to bar", "Renamed.", failing),
            Some(ReviewReason::FailingCheck)
        );
        assert_eq!(
            reason(
                "rename foo to bar",
                "Done, but the docs are still TODO.",
                CHECKED
            ),
            Some(ReviewReason::HedgedAnswer)
        );
    }

    #[test]
    fn ordinary_summaries_are_not_hedges() {
        for answer in [
            "No tests were deleted or skipped.",
            "The remaining workers now exit on error.",
            "I have not pushed; say if you want the branch pushed.",
            "Malformed lines are skipped, as before.",
            "Fixed the digit buffer reset; tests pass.",
        ] {
            assert!(!answer_looks_incomplete(answer), "{answer}");
        }
        for answer in [
            "Done, but the docs are still TODO.",
            "I couldn't run the tests.",
            "Not yet verified: the speedup.",
            "The change is untested.",
        ] {
            assert!(answer_looks_incomplete(answer), "{answer}");
        }
    }

    #[test]
    fn diff_size_counts_files_lines_and_untracked() {
        let (dir, start) = repo();
        let large = |base: Option<&str>| diff_is_large(dir.path(), base);
        assert!(!large(Some(&start)));
        std::fs::write(dir.path().join("a.txt"), "2\n").unwrap();
        assert!(!large(Some(&start)), "one small edit");
        for n in 0..2 {
            std::fs::write(dir.path().join(format!("new{n}.txt")), "x\n").unwrap();
        }
        assert!(large(Some(&start)), "three files touched");
        git(dir.path(), &["checkout", "-q", "a.txt"]);
        std::fs::remove_file(dir.path().join("new0.txt")).unwrap();
        std::fs::remove_file(dir.path().join("new1.txt")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(80)).unwrap();
        assert!(large(Some(&start)), "many lines in one file");
    }

    #[test]
    fn a_committed_change_is_measured_against_the_turn_start_not_head() {
        let (dir, start) = repo();
        for n in 0..3 {
            std::fs::write(dir.path().join(format!("f{n}.txt")), "x\n").unwrap();
        }
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "the model's own commit"]);
        assert!(
            !diff_is_large(dir.path(), None),
            "against HEAD the committed work has vanished"
        );
        assert!(diff_is_large(dir.path(), Some(&start)));
        assert_eq!(
            review_reason(
                "rename foo to bar",
                "Done.",
                dir.path(),
                Some(&start),
                CHECKED
            ),
            Some(ReviewReason::LargeDiff)
        );
    }

    #[test]
    fn an_unknown_turn_start_falls_back_to_head() {
        let (dir, _) = repo();
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(80)).unwrap();
        assert!(diff_is_large(
            dir.path(),
            Some("0000000000000000000000000000000000000000")
        ));
        assert!(diff_is_large(dir.path(), None));
    }
}
