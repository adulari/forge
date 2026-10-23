//! When the bridge's one-shot completeness review earns its extra model round trips.
//!
//! The review re-drives the whole bridged loop once more (a full request per step, each carrying
//! the entire tool surface). Measured over 71 bridged turns it changed content twice, and on the
//! Claude Code parity suites it fired on 70-100% of tasks, added 10-15% of the wall time and
//! changed no outcome once a check had passed after the last edit. So it only runs when something
//! about the turn makes a missed requirement likely: a check is still failing, the answer admits
//! unfinished work, or nothing was checked after the last edit and the request names several
//! requirements or the change is large.
//!
//! Size and content are measured against the commit the turn started from, not `HEAD`: a model
//! that commits its own work leaves `git diff HEAD` empty. Work an earlier turn left uncommitted
//! is subtracted ([`TurnBaseline`]), and the byproducts of running the tests (`__pycache__/`,
//! `node_modules/`) are not changes at all.

use forge_types::Message;

const LARGE_DIFF_FILES: usize = 3;
const LARGE_DIFF_LINES: usize = 60;
/// Prose sentences that make a request long enough for an unchecked change to miss a requirement.
const LONG_REQUEST_SENTENCES: usize = 5;

/// What the tool ledger saw this turn.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CheckEvidence {
    /// A test, lint, typecheck or build passed after the last successful edit.
    pub(crate) passed_after_last_edit: bool,
    /// A check failed and has not passed since.
    pub(crate) failing: bool,
    /// A file tool changed something, as opposed to commands that merely ran. Untracked files
    /// count toward the size of the change only then: a server's state directory is not an edit.
    pub(crate) authored: bool,
}

/// Why a turn earns the review. Only the first match is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewReason {
    FailingCheck,
    HedgedAnswer,
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

/// What the tree already held when the turn began: the uncommitted work of earlier turns. A
/// review is about this turn's change, and in a session whose earlier turns left their edits
/// uncommitted `git diff <turn-start-commit>` is large before the model has done anything, so
/// every later turn used to earn the review.
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnBaseline {
    /// Tracked files that differed from the turn's starting commit: added and removed lines.
    tracked: std::collections::HashMap<String, (usize, usize)>,
    /// Untracked work files and what they held.
    untracked: std::collections::HashMap<String, (u64, u64)>,
}

/// More untracked files than this and the baseline stops listing them: a tree that crowded is
/// not one where counting a few more changes anything.
const BASELINE_UNTRACKED_CAP: usize = 500;

impl TurnBaseline {
    pub(crate) fn capture(root: &std::path::Path, base: Option<&str>) -> Self {
        let mut baseline = Self::default();
        if let Some(rows) = numstat(root, base) {
            baseline.tracked = rows
                .into_iter()
                .map(|(path, added, removed)| (path, (added, removed)))
                .collect();
        }
        baseline.untracked = crate::working_tree::untracked_work_files(root)
            .unwrap_or_default()
            .into_iter()
            .take(BASELINE_UNTRACKED_CAP)
            .map(|path| {
                let digest = crate::working_tree::untracked_digest(root, &path);
                (path, digest)
            })
            .collect();
        baseline
    }

    /// The paths this turn changed: tracked files whose diff against `base` is not what the
    /// baseline recorded, and untracked work files that are new or different. `None` is a git
    /// failure, or a row that is no usable path: a rename (`old => new`) or a quoted name.
    pub(crate) fn changed_paths(
        &self,
        root: &std::path::Path,
        base: Option<&str>,
    ) -> Option<Vec<String>> {
        let mut paths = Vec::new();
        for (path, added, removed) in numstat(root, base)? {
            if path.contains(" => ") || path.starts_with('"') {
                return None;
            }
            if self.tracked.get(&path) != Some(&(added, removed)) {
                paths.push(path);
            }
        }
        for path in crate::working_tree::untracked_work_files(root).unwrap_or_default() {
            let digest = crate::working_tree::untracked_digest(root, &path);
            if self.untracked.get(&path) != Some(&digest) {
                paths.push(path);
            }
        }
        Some(paths)
    }
}

fn git_text(root: &std::path::Path, args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `git diff <base> --numstat` as `(path, added, removed)`; binary files count as 0/0. `None` is
/// a git failure.
fn numstat(root: &std::path::Path, base: Option<&str>) -> Option<Vec<(String, usize, usize)>> {
    let raw = base
        .and_then(|base| git_text(root, &["diff", base, "--numstat"]))
        .or_else(|| git_text(root, &["diff", "HEAD", "--numstat"]))?;
    Some(
        raw.lines()
            .filter_map(|row| {
                let mut cols = row.splitn(3, '\t');
                let added = cols.next()?.parse().unwrap_or(0);
                let removed = cols.next()?.parse().unwrap_or(0);
                Some((cols.next()?.to_string(), added, removed))
            })
            .collect(),
    )
}

/// How much this turn changed the working tree: files and changed lines relative to `base` (the
/// turn's starting commit, else `HEAD`), committed or not, minus what `baseline` says was already
/// there and counting untracked files that are the model's work. `None` is a git failure.
fn diff_size(
    root: &std::path::Path,
    base: Option<&str>,
    baseline: &TurnBaseline,
    count_untracked: bool,
) -> Option<(usize, usize)> {
    let (mut files, mut lines) = (0usize, 0usize);
    for (path, added, removed) in numstat(root, base)? {
        match baseline.tracked.get(&path) {
            Some(&(was_added, was_removed)) if (added, removed) == (was_added, was_removed) => {}
            Some(&(was_added, was_removed)) => {
                files += 1;
                lines += added.abs_diff(was_added) + removed.abs_diff(was_removed);
            }
            None => {
                files += 1;
                lines += added + removed;
            }
        }
    }
    if count_untracked {
        for path in crate::working_tree::untracked_work_files(root).unwrap_or_default() {
            let digest = crate::working_tree::untracked_digest(root, &path);
            if baseline.untracked.get(&path) != Some(&digest) {
                files += 1;
            }
        }
    }
    Some((files, lines))
}

/// True when this turn changed the tree by several files or many lines. A git failure reads as
/// "not large"; the other signals still apply.
pub(crate) fn diff_is_large(
    root: &std::path::Path,
    base: Option<&str>,
    baseline: &TurnBaseline,
    count_untracked: bool,
) -> bool {
    diff_size(root, base, baseline, count_untracked)
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
    baseline: &TurnBaseline,
    evidence: CheckEvidence,
) -> Option<ReviewReason> {
    if evidence.failing {
        return Some(ReviewReason::FailingCheck);
    }
    if answer_looks_incomplete(answer) {
        return Some(ReviewReason::HedgedAnswer);
    }
    if evidence.passed_after_last_edit {
        return None;
    }
    let prose = request_prose(request);
    if list_items(&prose) >= 2 || prose_sentences(&prose) >= LONG_REQUEST_SENTENCES {
        return Some(ReviewReason::UnverifiedMultiRequirement);
    }
    diff_is_large(root, turn_start, baseline, evidence.authored).then_some(ReviewReason::LargeDiff)
}

impl crate::Session {
    /// Whether this turn ran a tool or changed anything, as opposed to answering from the
    /// conversation. A question-only turn must not re-verify the all-Done list an earlier turn
    /// left: a memory probe asked after a finished plan was answered correctly, then sent to run
    /// the test suite, and finished with that run's one-line result as its answer. Both the
    /// direct and the bridge completion gates ask this.
    pub(crate) fn turn_did_work(&self, tools_ran: &std::sync::atomic::AtomicU64) -> bool {
        tools_ran.load(std::sync::atomic::Ordering::Relaxed) > 0 || self.mutations_this_turn > 0
    }

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
                authored: ledger.authored(),
            }
        };
        let reason = review_reason(
            latest_request(&self.transcript),
            answer,
            self.workspace_root(),
            self.turn_start_head(),
            &self.turn_start_baseline,
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
        authored: true,
    };
    const UNCHECKED: CheckEvidence = CheckEvidence {
        passed_after_last_edit: false,
        failing: false,
        authored: true,
    };

    fn reason(request: &str, answer: &str, evidence: CheckEvidence) -> Option<ReviewReason> {
        let (dir, start) = repo();
        review_reason(
            request,
            answer,
            dir.path(),
            Some(&start),
            &TurnBaseline::default(),
            evidence,
        )
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
    fn listed_requirements_are_reviewed_only_when_nothing_checked_them() {
        let two = "Fix it:\n1. reject dotted blueprint names\n2. reject dotted endpoint names";
        assert_eq!(
            reason(two, "Done.", UNCHECKED),
            Some(ReviewReason::UnverifiedMultiRequirement)
        );
        assert_eq!(reason(two, "Done.", CHECKED), None);
        let three = format!("{two}\n3. reject dotted view names");
        assert_eq!(reason(&three, "Done.", CHECKED), None);
    }

    #[test]
    fn a_failing_check_or_a_hedge_always_earns_the_review() {
        let failing = CheckEvidence {
            passed_after_last_edit: false,
            failing: true,
            authored: true,
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
        let large =
            |base: Option<&str>| diff_is_large(dir.path(), base, &TurnBaseline::default(), true);
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
            !diff_is_large(dir.path(), None, &TurnBaseline::default(), true),
            "against HEAD the committed work has vanished"
        );
        assert!(diff_is_large(
            dir.path(),
            Some(&start),
            &TurnBaseline::default(),
            true
        ));
        let large_change = |evidence| {
            review_reason(
                "rename foo to bar",
                "Done.",
                dir.path(),
                Some(&start),
                &TurnBaseline::default(),
                evidence,
            )
        };
        assert_eq!(large_change(UNCHECKED), Some(ReviewReason::LargeDiff));
        assert_eq!(
            large_change(CHECKED),
            None,
            "a check passed after the last edit"
        );
    }

    #[test]
    fn an_unknown_turn_start_falls_back_to_head() {
        let (dir, _) = repo();
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(80)).unwrap();
        let none = TurnBaseline::default();
        assert!(diff_is_large(
            dir.path(),
            Some("0000000000000000000000000000000000000000"),
            &none,
            true
        ));
        assert!(diff_is_large(dir.path(), None, &none, true));
    }

    #[test]
    fn test_run_byproducts_are_not_changed_files() {
        let (dir, start) = repo();
        std::fs::write(dir.path().join("a.txt"), "2\n").unwrap();
        for path in [
            "pkg/__pycache__/a.cpython-312.pyc",
            "pkg/__pycache__/b.cpython-312.pyc",
            "web/node_modules/x/index.js",
        ] {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, "x").unwrap();
        }
        assert!(
            !diff_is_large(dir.path(), Some(&start), &TurnBaseline::default(), true),
            "one edited file and three cache files is a one-file change"
        );
        std::fs::write(dir.path().join("new.py"), "x\n").unwrap();
        std::fs::write(dir.path().join("other.py"), "x\n").unwrap();
        assert!(diff_is_large(
            dir.path(),
            Some(&start),
            &TurnBaseline::default(),
            true
        ));
    }

    #[test]
    fn untracked_files_count_only_when_the_turn_authored_something() {
        let (dir, start) = repo();
        for name in [
            ".state/issued.log",
            ".state/shutdown.log",
            "report-token.txt",
        ] {
            let full = dir.path().join(name);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, "x").unwrap();
        }
        let none = TurnBaseline::default();
        assert!(diff_is_large(dir.path(), Some(&start), &none, true));
        assert!(
            !diff_is_large(dir.path(), Some(&start), &none, false),
            "what a started service wrote is not three edited files"
        );
        // A tracked edit is always an edit.
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(80)).unwrap();
        assert!(diff_is_large(dir.path(), Some(&start), &none, false));
    }

    #[test]
    fn work_an_earlier_turn_left_uncommitted_is_not_this_turns_change() {
        let (dir, start) = repo();
        for name in ["one.txt", "two.txt", "three.txt"] {
            std::fs::write(dir.path().join(name), "v1\n").unwrap();
        }
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "earlier"]);
        let start = {
            let _ = start;
            head(dir.path())
        };
        for name in ["one.txt", "two.txt", "three.txt"] {
            std::fs::write(dir.path().join(name), "v2\nmore\n").unwrap();
        }
        std::fs::write(dir.path().join("notes.md"), "scratch").unwrap();
        let baseline = TurnBaseline::capture(dir.path(), Some(&start));
        assert!(
            diff_is_large(dir.path(), Some(&start), &TurnBaseline::default(), true),
            "without a baseline the leftovers alone read as a large change"
        );
        assert!(!diff_is_large(dir.path(), Some(&start), &baseline, true));

        // This turn's own edits still count, on top of the leftovers.
        std::fs::write(dir.path().join("a.txt"), "edited\n").unwrap();
        std::fs::write(dir.path().join("one.txt"), "v3\nmore\nmore\n").unwrap();
        assert!(!diff_is_large(dir.path(), Some(&start), &baseline, true));
        std::fs::write(dir.path().join("b.py"), "x\n").unwrap();
        assert!(diff_is_large(dir.path(), Some(&start), &baseline, true));
        std::fs::write(dir.path().join("notes.md"), "scratch, then rewritten").unwrap();
        std::fs::remove_file(dir.path().join("b.py")).unwrap();
        assert!(
            diff_is_large(dir.path(), Some(&start), &baseline, true),
            "a leftover file the turn rewrote counts again"
        );
    }
}
