//! The post-edit `[lsp diagnostics]` block as a delta against what the model already saw.
//!
//! A benchmark on a small Python repo measured one block per edit at 5–20k characters: a strict
//! type checker reports the file's whole backlog (unannotated parameters, unknown member types)
//! after every write, the model treats it as pre-existing noise, and the block is then re-sent with
//! the transcript on every later request. Identical blocks are already deduplicated on the wire
//! (`system_context::normalize_system_context`), but a block after a second edit is never identical
//! to the first: line numbers shift, one warning changes. So the dedup saved nothing here.
//!
//! What this module sends instead, per file and per session:
//! - first touch: diagnostics on or near the edited lines, every error, and a total count;
//! - later touches: only diagnostics that are new since the previous snapshot of that file (old
//!   lines are mapped through a line diff so a shifted warning is not "new"), plus a resolved count;
//! - errors the edit introduced (in or near the changed lines, or newly appearing) are always shown,
//!   past the soft cap, because those are the ones the model must fix this turn;
//! - everything else is capped and sorted deterministically so the block is stable for the cache.
//!
//! Nothing that drives correctness reads this block: the verification ledger, the phantom-edit
//! gate, the failure-loop guards and auto-lint all observe the tool result or run their own
//! commands. It is advice to the model and nothing else.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;

use forge_lsp::types::{Diagnostic, DiagnosticSeverity};

/// Lines either side of a changed range that still count as "near the edit".
pub(crate) const NEAR_LINES: u32 = 3;
/// Soft cap on the rendered block. Introduced errors may exceed it, up to [`HARD_CAP_CHARS`].
pub(crate) const SOFT_CAP_CHARS: usize = 2_000;
/// Even introduced errors stop here: an edit that breaks a file's syntax can produce hundreds.
pub(crate) const HARD_CAP_CHARS: usize = 8_000;
const ERROR_MESSAGE_CHARS: usize = 300;
const OTHER_MESSAGE_CHARS: usize = 160;

/// What a file looked like, and what the server said about it, the last time the model was told.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    text: String,
    diagnostics: Vec<Diagnostic>,
}

/// Per-file snapshots for one session. Cleared when the transcript is rewritten, because the model
/// may no longer have seen the blocks the snapshots stand for.
#[derive(Debug, Default)]
pub(crate) struct Baselines {
    files: HashMap<PathBuf, Snapshot>,
}

impl Baselines {
    pub(crate) fn clear(&mut self) {
        self.files.clear();
    }

    /// Render the block for `path` after an edit and remember the new snapshot.
    ///
    /// `before` is the file's text immediately before this write (`None` when it did not exist),
    /// `after` its text now.
    pub(crate) fn observe(
        &mut self,
        path: PathBuf,
        label: &str,
        before: Option<&str>,
        after: String,
        diagnostics: Vec<Diagnostic>,
    ) -> Option<String> {
        let prior = self.files.get(&path);
        let block = render(label, prior, before, &after, &diagnostics);
        self.files.insert(
            path,
            Snapshot {
                text: after,
                diagnostics,
            },
        );
        block
    }
}

/// Line ranges of `after` that this edit wrote. A pure deletion is an empty range at the point
/// where the lines used to be, so diagnostics right next to it still count as near.
pub(crate) fn changed_ranges(before: Option<&str>, after: &str) -> Vec<Range<u32>> {
    let Some(before) = before else {
        return std::iter::once(0..line_count(after).max(1)).collect();
    };
    diff_ops(before, after)
        .into_iter()
        .filter_map(|op| match op {
            similar::DiffOp::Equal { .. } => None,
            similar::DiffOp::Delete { new_index, .. } => Some(new_index..new_index),
            similar::DiffOp::Insert {
                new_index, new_len, ..
            }
            | similar::DiffOp::Replace {
                new_index, new_len, ..
            } => Some(new_index..new_index + new_len),
        })
        .map(|r| to_u32(r.start)..to_u32(r.end))
        .collect()
}

/// Where each line of `before` sits in `after`, for lines the diff kept unchanged.
fn line_map(before: &str, after: &str) -> HashMap<u32, u32> {
    let mut map = HashMap::new();
    for op in diff_ops(before, after) {
        if let similar::DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for i in 0..len {
                map.insert(to_u32(old_index + i), to_u32(new_index + i));
            }
        }
    }
    map
}

fn diff_ops(before: &str, after: &str) -> Vec<similar::DiffOp> {
    similar::TextDiffConfig::default()
        .timeout(std::time::Duration::from_millis(250))
        .diff_lines(before, after)
        .ops()
        .to_vec()
}

fn line_count(text: &str) -> u32 {
    to_u32(text.lines().count())
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Lines between a diagnostic and the nearest changed range (0 = inside one).
fn distance(line: u32, ranges: &[Range<u32>]) -> u32 {
    ranges
        .iter()
        .map(|r| {
            if line < r.start {
                r.start - line
            } else if line >= r.end {
                // An empty range (deletion) at `start` is 0 lines from the line now at `start`.
                (line + 1).saturating_sub(r.end.max(r.start + 1))
            } else {
                0
            }
        })
        .min()
        .unwrap_or(u32::MAX)
}

fn severity_rank(severity: &DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Information => 2,
        DiagnosticSeverity::Hint => 3,
    }
}

type Key<'a> = (u8, Option<&'a str>, &'a str);

fn key(d: &Diagnostic) -> Key<'_> {
    (
        severity_rank(&d.severity),
        d.code.as_deref(),
        d.message.as_str(),
    )
}

/// Indices of `current` that the prior snapshot did not have, and how many prior ones are gone.
///
/// A prior diagnostic matches a current one with the same severity, code and message on the line
/// its old line maps to. Prior diagnostics whose line the edit rewrote match by content alone, so a
/// warning on a line the model touched without fixing is not reported as new.
fn new_since(prior: &Snapshot, after: &str, current: &[Diagnostic]) -> (Vec<bool>, usize) {
    let map = line_map(&prior.text, after);
    let mut anchored: HashMap<(Key<'_>, u32), usize> = HashMap::new();
    let mut floating: HashMap<Key<'_>, usize> = HashMap::new();
    for d in &prior.diagnostics {
        match map.get(&d.line) {
            Some(&line) => *anchored.entry((key(d), line)).or_default() += 1,
            None => *floating.entry(key(d)).or_default() += 1,
        }
    }
    let mut is_new = vec![true; current.len()];
    for (i, d) in current.iter().enumerate() {
        if let Some(n) = anchored.get_mut(&(key(d), d.line)).filter(|n| **n > 0) {
            *n -= 1;
            is_new[i] = false;
        }
    }
    for (i, d) in current.iter().enumerate() {
        if is_new[i] {
            if let Some(n) = floating.get_mut(&key(d)).filter(|n| **n > 0) {
                *n -= 1;
                is_new[i] = false;
            }
        }
    }
    let resolved = anchored.values().sum::<usize>() + floating.values().sum::<usize>();
    (is_new, resolved)
}

struct Candidate<'a> {
    diagnostic: &'a Diagnostic,
    distance: u32,
    must_show: bool,
}

/// The block for one edit, or `None` when there is nothing the model has not already been told.
pub(crate) fn render(
    label: &str,
    prior: Option<&Snapshot>,
    before: Option<&str>,
    after: &str,
    current: &[Diagnostic],
) -> Option<String> {
    let ranges = changed_ranges(before, after);
    let (is_new, resolved) = match prior {
        Some(prior) => new_since(prior, after, current),
        None => (vec![false; current.len()], 0),
    };
    let mut shown: Vec<Candidate<'_>> = current
        .iter()
        .zip(&is_new)
        .filter_map(|(d, &new)| {
            let distance = distance(d.line, &ranges);
            let near = distance <= NEAR_LINES;
            let error = d.severity == DiagnosticSeverity::Error;
            let show = match prior {
                Some(_) => new || (error && near),
                // No snapshot yet: every error, plus anything else around the edit.
                None => near || error,
            };
            show.then_some(Candidate {
                diagnostic: d,
                distance,
                must_show: error && (near || new),
            })
        })
        .collect();
    let new_count = is_new.iter().filter(|n| **n).count();
    if prior.is_some() && shown.is_empty() && resolved == 0 {
        return None;
    }
    if prior.is_none() && current.is_empty() {
        return None;
    }
    shown.sort_by(|a, b| {
        let (da, db) = (a.diagnostic, b.diagnostic);
        b.must_show
            .cmp(&a.must_show)
            .then(severity_rank(&da.severity).cmp(&severity_rank(&db.severity)))
            .then(a.distance.cmp(&b.distance))
            .then(da.line.cmp(&db.line))
            .then(da.character.cmp(&db.character))
            .then(da.code.cmp(&db.code))
            .then(da.message.cmp(&db.message))
    });

    let errors = current
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let mut out = match prior {
        Some(_) => format!(
            "[lsp diagnostics] {label}: {new_count} new, {resolved} resolved since your last edit \
             ({} total, {})",
            current.len(),
            errors_label(errors)
        ),
        None => format!(
            "[lsp diagnostics] {label}: {} total ({}); showing errors and those near your edit",
            current.len(),
            errors_label(errors)
        ),
    };
    let mut omitted = 0usize;
    for c in &shown {
        let line = format_line(c.diagnostic);
        let cap = if c.must_show {
            HARD_CAP_CHARS
        } else {
            SOFT_CAP_CHARS
        };
        if out.len() + 1 + line.len() > cap {
            omitted += 1;
            continue;
        }
        out.push('\n');
        out.push_str(&line);
    }
    if omitted > 0 {
        out.push_str(&format!(
            "\n  +{omitted} more (run the linter or type checker to see all)"
        ));
    }
    Some(out)
}

fn errors_label(n: usize) -> String {
    if n == 1 {
        "1 error".to_string()
    } else {
        format!("{n} errors")
    }
}

/// One diagnostic as a single line: the file is named once in the header, not on every line.
fn format_line(d: &Diagnostic) -> String {
    let limit = if d.severity == DiagnosticSeverity::Error {
        ERROR_MESSAGE_CHARS
    } else {
        OTHER_MESSAGE_CHARS
    };
    let message: String = d
        .message
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" — ");
    let message = if message.chars().count() > limit {
        let cut: String = message.chars().take(limit).collect();
        format!("{cut}…")
    } else {
        message
    };
    let code = d
        .code
        .as_deref()
        .map(|c| format!(" ({c})"))
        .unwrap_or_default();
    format!(
        "  L{}:{} {}{code} {message}",
        d.line.saturating_add(1),
        d.character.saturating_add(1),
        d.severity.as_str()
    )
}

#[cfg(test)]
#[path = "tests/lsp_delta.rs"]
mod tests;
