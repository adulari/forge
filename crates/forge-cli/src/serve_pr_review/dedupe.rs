//! Collapses near-duplicate findings before the cap, so one issue reported twice with different
//! wording does not take two of the few slots.

use std::collections::BTreeSet;

use super::{rank, ReviewFinding};

const NEAR_LINES: u32 = 3;
const SAME_TITLE: f64 = 0.6;
const SAME_TITLE_NEAR: f64 = 0.25;

fn words(title: &str) -> BTreeSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_lowercase)
        .collect()
}

fn overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

fn near(a: Option<u32>, b: Option<u32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) <= NEAR_LINES,
        (None, None) => true,
        _ => false,
    }
}

fn same_issue(a: &ReviewFinding, b: &ReviewFinding) -> bool {
    if a.file != b.file {
        return false;
    }
    let similarity = overlap(&words(&a.title), &words(&b.title));
    similarity >= SAME_TITLE || (near(a.line, b.line) && similarity >= SAME_TITLE_NEAR)
}

/// Keeps the first of each group of duplicates, upgraded to the group's highest severity.
pub(super) fn dedupe(findings: Vec<ReviewFinding>) -> Vec<ReviewFinding> {
    let mut kept: Vec<ReviewFinding> = Vec::new();
    for f in findings {
        match kept.iter_mut().find(|k| same_issue(k, &f)) {
            Some(k) => {
                if rank(&f.severity) > rank(&k.severity) {
                    *k = f;
                }
            }
            None => kept.push(f),
        }
    }
    kept
}
