//! Fingerprinting a tool result: which of its lines are new, what a failure is, how noisy it is.

use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

/// One-line results at most this long are treated as status messages (`no matches for 'x'`,
/// `ok`): their quoted parts are masked so varying only the query does not read as new.
const SHORT_RESULT_CHARS: usize = 160;
/// Longest label or error excerpt kept for a progress report.
pub(super) const EXCERPT_CHARS: usize = 60;

pub(crate) fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

pub(super) fn hash_lines(lines: &[u64]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn excerpt(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= EXCERPT_CHARS {
        return flat;
    }
    let head: String = flat.chars().take(EXCERPT_CHARS).collect();
    format!("{head}…")
}

/// Fingerprints of the lines a result printed, blank lines dropped.
///
/// The shell's `exit N in Xms` header carries the run's timing, so its digits are always masked:
/// a silent command (`true`, `git add`) whose whole result is that header would otherwise look
/// like new output every time it ran. `mask_all` masks the digits of every line, for failures:
/// an attempt counter, a timestamp or a port in an error message is noise, and a retry loop that
/// differs only in those must not read as new information. A successful result keeps its
/// numbers, because a poll whose counter moves is moving.
pub(super) fn line_fingerprints(result: &str, mask_all: bool) -> Vec<u64> {
    let mut lines = result.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next();
    let rest: Vec<&str> = lines.collect();
    let mut out = Vec::with_capacity(rest.len() + 1);
    let single_short = rest.is_empty() && first.is_some_and(|l| l.len() <= SHORT_RESULT_CHARS);
    if let Some(first) = first {
        let header = if mask_all || first.starts_with("shell:") {
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
    out.extend(rest.into_iter().map(|line| {
        if mask_all {
            hash_str(&mask_digits(line))
        } else {
            hash_str(line)
        }
    }));
    out
}

/// Every run of digits becomes one `#`: `3ms` and `41ms` are the same timing.
pub(super) fn mask_digits(s: &str) -> String {
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

/// A line whose content changes from run to run for reasons that say nothing about the outcome:
/// a duration, a clock time, a date, a long hex id.
fn is_volatile(line: &str) -> bool {
    static VOLATILE: OnceLock<regex::Regex> = OnceLock::new();
    VOLATILE
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)\d+(\.\d+)?\s?(ms|µs|us|ns|s|secs?|seconds|mins?|minutes)\b|\d{1,2}:\d{2}(:\d{2})?|\d{4}-\d{2}-\d{2}|\b0x[0-9a-f]{6,}\b|\b[0-9a-f]{8,}\b",
            )
            .expect("volatile-line pattern")
        })
        .is_match(line)
}

/// What a failing result says once its run-to-run noise is taken out: the set of its lines that
/// are not durations, times or ids. Two runs of a check that fail the same way give the same
/// value however long each took; a failure that moved (a different assertion, a changed count)
/// gives another.
pub(super) fn stable_fingerprint(result: &str) -> u64 {
    let mut lines: Vec<u64> = result
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("shell:") && !is_volatile(l))
        .map(hash_str)
        .collect();
    lines.sort_unstable();
    lines.dedup();
    hash_lines(&lines)
}

/// Whether a result has anything to say beyond the shell's own header.
pub(super) fn has_body(result: &str) -> bool {
    result.lines().filter(|l| !l.trim().is_empty()).count() > 1
}

/// The line of a failed result that says what went wrong: the first one, unless that is only
/// the shell's `exit N in Xms` header, in which case the line after it. A failure with nothing
/// but the header says nothing (`grep` found no match, `test -f` was false), so it has no text.
pub(super) fn error_text(result: &str) -> Option<String> {
    let mut lines = result.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?;
    if first.starts_with("shell:") {
        return lines.next().map(excerpt);
    }
    Some(excerpt(first))
}
