//! The human-readable report: one row per category with the score after compaction and after
//! the resume, then the compaction record and the spend.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::scenario::Category;
use super::score::{PhaseScore, Report, Tally};

fn cell(t: Option<&Tally>) -> String {
    match t {
        Some(t) if t.total > 0 => {
            format!(
                "{:>5} {:>5.1}%",
                format!("{}/{}", t.passed, t.total),
                t.pct()
            )
        }
        _ => "-".to_string(),
    }
}

pub(super) fn table(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "forge bench recall | model {} | {} turns | session {}",
        report.model,
        report.scenario_turns,
        report.session_id.as_deref().unwrap_or("-")
    );
    let _ = writeln!(
        out,
        "{:<14}{:<18}after-resume",
        "category", "after-compaction"
    );
    let after: &PhaseScore = &report.after_compaction;
    let resume = report.after_resume.as_ref();
    let categories: BTreeSet<Category> = after.by_category.keys().copied().collect();
    for category in categories {
        let _ = writeln!(
            out,
            "{:<14}{:<18}{}",
            category.label(),
            cell(after.by_category.get(&category)),
            if resume.is_some() {
                cell(resume.and_then(|r| r.by_category.get(&category)))
            } else {
                "skipped".to_string()
            }
        );
    }
    let _ = writeln!(
        out,
        "{:<14}{:<18}{}",
        "TOTAL",
        cell(Some(&after.total)),
        if resume.is_some() {
            cell(resume.map(|r| &r.total))
        } else {
            "skipped".to_string()
        }
    );
    for (zone, tally) in &after.by_zone {
        let _ = writeln!(
            out,
            "  {:<12}{:<18}{}",
            format!("{} facts", zone.label()),
            cell(Some(tally)),
            cell(resume.and_then(|r| r.by_zone.get(zone)))
        );
    }
    for c in &report.compactions {
        match &c.error {
            None => {
                let _ = writeln!(
                    out,
                    "compaction {}: {} -> {} messages",
                    c.label, c.messages_before, c.messages_after
                );
            }
            Some(e) => {
                let _ = writeln!(out, "compaction {} FAILED: {e}", c.label);
            }
        }
    }
    let u = &report.usage;
    let _ = writeln!(
        out,
        "tokens in {} (cached {}) out {} reasoning {} | {} model calls, {} tool calls | cost ${:.4} | wall {:.0}s",
        u.input_tokens,
        u.cached_input_tokens,
        u.output_tokens,
        u.reasoning_tokens,
        u.model_calls,
        u.tool_calls,
        u.cost_usd,
        report.wall_secs
    );
    let assisted = after.tool_assisted + resume.map_or(0, |r| r.tool_assisted);
    if assisted > 0 {
        let _ = writeln!(
            out,
            "note: {assisted} memory question(s) used tools despite the instruction; those scores may be inflated"
        );
    }
    if report.failed_turns > 0 {
        let _ = writeln!(out, "note: {} scenario turn(s) failed", report.failed_turns);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::scenario::Zone;
    use super::super::score::{score_phase, CompactionRecord, Phase, ProbeResult, UsageTotals};
    use super::*;

    fn probe(category: Category, zone: Zone, phase: Phase, passed: bool) -> ProbeResult {
        ProbeResult {
            id: "x".into(),
            category,
            zone,
            phase,
            passed,
            observed: String::new(),
            tool_calls: 0,
            error: None,
        }
    }

    #[test]
    fn the_table_lists_categories_totals_compactions_and_spend() {
        let probes = vec![
            probe(Category::Number, Zone::Plant, Phase::AfterCompaction, true),
            probe(Category::Number, Zone::Late, Phase::AfterCompaction, false),
            probe(Category::Number, Zone::Plant, Phase::AfterResume, true),
        ];
        let report = Report {
            model: "m".into(),
            turns_scale: 1.0,
            scenario_turns: 23,
            failed_turns: 0,
            compactions: vec![CompactionRecord {
                label: "mid-scenario",
                messages_before: 60,
                messages_after: 9,
                error: None,
            }],
            resumed: true,
            after_compaction: score_phase(&probes, Phase::AfterCompaction),
            after_resume: Some(score_phase(&probes, Phase::AfterResume)),
            usage: UsageTotals {
                input_tokens: 1000,
                cost_usd: 0.5,
                ..UsageTotals::default()
            },
            wall_secs: 12.0,
            session_id: Some("abc".into()),
            workdir: None,
            probes,
        };
        let text = table(&report);
        assert!(text.contains("number          1/2  50.0%"), "{text}");
        assert!(text.contains("TOTAL"), "{text}");
        assert!(text.contains("  1/1 100.0%"), "{text}");
        assert!(
            text.contains("compaction mid-scenario: 60 -> 9 messages"),
            "{text}"
        );
        assert!(text.contains("cost $0.5000"), "{text}");
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["after_compaction"]["by_category"]["number"]["passed"],
            1
        );
        assert_eq!(json["after_resume"]["total"]["total"], 1);
    }

    #[test]
    fn a_skipped_resume_is_labelled_not_scored() {
        let probes = vec![probe(
            Category::Pending,
            Zone::Plant,
            Phase::AfterCompaction,
            true,
        )];
        let report = Report {
            model: "m".into(),
            turns_scale: 1.0,
            scenario_turns: 1,
            failed_turns: 1,
            compactions: vec![],
            resumed: false,
            after_compaction: score_phase(&probes, Phase::AfterCompaction),
            after_resume: None,
            usage: UsageTotals::default(),
            wall_secs: 0.0,
            session_id: None,
            workdir: None,
            probes,
        };
        let text = table(&report);
        assert!(text.contains("skipped"), "{text}");
        assert!(text.contains("1 scenario turn(s) failed"), "{text}");
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("after_resume").is_none());
    }
}
