//! When the bridge's one-shot completeness review earns its extra model round trips.
//!
//! The review re-drives the whole bridged loop once more (a full request per step, each carrying
//! the entire tool surface), so on a single small edit it only adds cost. It pays off when the
//! request lists several requirements, the change spans many files or lines, or the model's own
//! answer admits something is unfinished.

use forge_types::{Message, Role};

const LARGE_DIFF_FILES: usize = 3;
const LARGE_DIFF_LINES: usize = 60;

/// Distinct-requirement heuristic: a bulleted/numbered list of two or more items, or enough prose
/// sentences that the request can't be a single instruction.
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

/// The model's final answer hedges or admits unfinished work.
pub(crate) fn answer_looks_incomplete(answer: &str) -> bool {
    const MARKERS: &[&str] = &[
        "todo",
        "not yet",
        "remaining",
        "haven't",
        "have not",
        "didn't get to",
        "did not get to",
        "not implemented",
        "partial",
        "unable to",
        "couldn't",
        "could not",
        "left as is",
        "skipped",
        "still needs",
        "follow-up",
    ];
    let lower = answer.to_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

/// True when the working tree differs from HEAD by several files or many lines, counting
/// untracked files. `None`/git failure reads as "not large" — the other triggers still apply.
pub(crate) fn diff_is_large(root: &std::path::Path) -> bool {
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
    let (mut files, mut lines) = (0usize, 0usize);
    for row in git(&["diff", "HEAD", "--numstat"])
        .unwrap_or_default()
        .lines()
    {
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
    files >= LARGE_DIFF_FILES || lines >= LARGE_DIFF_LINES
}

/// The user's most recent request in the transcript.
fn latest_request(transcript: &[Message]) -> &str {
    transcript
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map_or("", |m| m.content.as_str())
}

impl crate::Session {
    /// Whether the bridge's completeness review is worth its extra round trip for this turn:
    /// the request lists several requirements, the change is large, or the answer hedges.
    pub(crate) fn review_warranted(&self, answer: &str) -> bool {
        review_warranted(
            latest_request(&self.transcript),
            answer,
            self.workspace_root(),
        )
    }
}

fn review_warranted(request: &str, answer: &str, root: &std::path::Path) -> bool {
    request_lists_multiple_requirements(request)
        || answer_looks_incomplete(answer)
        || diff_is_large(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_small_instruction_is_not_multi_requirement() {
        assert!(!request_lists_multiple_requirements(
            "parse_duration(\"1h30m\") returns 11400 instead of 5400. Find the root cause and fix it, and add a regression test for it."
        ));
        assert!(!request_lists_multiple_requirements("rename foo to bar"));
    }

    #[test]
    fn lists_and_long_prose_are_multi_requirement() {
        assert!(request_lists_multiple_requirements(
            "Fix it:\n1. reject dotted blueprint names\n2. reject dotted endpoint names"
        ));
        assert!(request_lists_multiple_requirements(
            "Do this:\n- add the flag\n- document it"
        ));
        assert!(request_lists_multiple_requirements(
            "Make the retry delay a setting. Default it to 5s and use it in Store. Document it in the README."
        ));
    }

    #[test]
    fn hedging_answer_is_incomplete() {
        assert!(answer_looks_incomplete(
            "Done, but the docs are still TODO."
        ));
        assert!(answer_looks_incomplete("I couldn't run the tests."));
        assert!(!answer_looks_incomplete(
            "Fixed the digit buffer reset; tests pass."
        ));
    }

    #[test]
    fn diff_size_counts_files_lines_and_untracked() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q"]);
        std::fs::write(dir.path().join("a.txt"), "1\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        assert!(!diff_is_large(dir.path()));
        std::fs::write(dir.path().join("a.txt"), "2\n").unwrap();
        assert!(!diff_is_large(dir.path()), "one small edit");
        for n in 0..2 {
            std::fs::write(dir.path().join(format!("new{n}.txt")), "x\n").unwrap();
        }
        assert!(diff_is_large(dir.path()), "three files touched");
        run(&["checkout", "-q", "a.txt"]);
        std::fs::remove_file(dir.path().join("new0.txt")).unwrap();
        std::fs::remove_file(dir.path().join("new1.txt")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "x\n".repeat(80)).unwrap();
        assert!(diff_is_large(dir.path()), "many lines in one file");
    }
}
