//! How `forge mesh explain` shows the measured cost-per-solve signal: a suffix on each candidate
//! it moved, a block naming every measured candidate and why it did or did not count, and a JSON
//! object per row.

use forge_mesh::{CandidateRow, SolveCostSignal};

/// ` · $/solve +0.75` on a row the signal moved, empty otherwise.
pub(super) fn suffix(signal: Option<&SolveCostSignal>) -> String {
    signal
        .filter(|s| s.applied())
        .map_or_else(String::new, |s| format!(" · $/solve {:+.2}", s.adjustment))
}

/// The explanation block under the candidate table; empty when no candidate was measured.
pub(super) fn block(note: Option<String>, shown: &[&CandidateRow]) -> String {
    let Some(note) = note else {
        return String::new();
    };
    let mut out = format!("\n{note}\n");
    for c in shown {
        if let Some(s) = &c.row.solve_cost {
            out.push_str(&format!("  {:<38} {}\n", c.row.model, s.describe()));
        }
    }
    out
}

pub(super) fn json(signal: Option<&SolveCostSignal>) -> serde_json::Value {
    match signal {
        None => serde_json::Value::Null,
        Some(s) => serde_json::json!({
            "attempts": s.measured.attempts,
            "solved": s.measured.solved,
            "cost_usd": s.measured.cost_usd,
            "cost_per_solve": s.measured.cost_per_solve(),
            "reference": s.reference,
            "adjustment": s.adjustment,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_mesh::MeasuredSolveCost;

    fn signal(adjustment: f64) -> SolveCostSignal {
        SolveCostSignal {
            measured: MeasuredSolveCost {
                attempts: 24,
                solved: 12,
                cost_usd: 1.5,
            },
            reference: Some(0.2),
            adjustment,
        }
    }

    #[test]
    fn only_a_row_the_signal_moved_gets_a_suffix() {
        assert_eq!(suffix(Some(&signal(0.75))), " · $/solve +0.75");
        assert_eq!(suffix(Some(&signal(0.0))), "");
        assert_eq!(suffix(None), "");
    }

    #[test]
    fn json_carries_the_measurement_and_the_adjustment() {
        let v = json(Some(&signal(0.75)));
        assert_eq!(v["attempts"], 24);
        assert_eq!(v["cost_per_solve"], 0.125);
        assert_eq!(v["adjustment"], 0.75);
        assert!(json(None).is_null());
    }
}
