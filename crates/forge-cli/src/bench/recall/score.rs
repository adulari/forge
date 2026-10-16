//! Deterministic scoring and the report model: a probe passes or fails on regexes, results roll
//! up per category, per zone and in total, separately for each measurement phase.

use std::collections::BTreeMap;

use regex::RegexBuilder;
use serde::Serialize;

use super::scenario::{Category, Matcher, Zone};

impl Matcher {
    pub(super) fn accepts(&self, text: &str) -> bool {
        let hit = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .multi_line(true)
                .build()
                .map(|re| re.is_match(text))
                .unwrap_or(false)
        };
        self.all.iter().all(|p| hit(p)) && !self.none.iter().any(|p| hit(p))
    }

    /// Every pattern compiles; the scenario tests insist on it so a typo cannot silently fail a
    /// fact for every model.
    #[cfg(test)]
    pub(super) fn patterns_compile(&self) -> bool {
        self.all
            .iter()
            .chain(self.none.iter())
            .all(|p| RegexBuilder::new(p).build().is_ok())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    AfterCompaction,
    AfterResume,
}

impl Phase {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::AfterCompaction => "after-compaction",
            Self::AfterResume => "after-resume",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ProbeResult {
    pub id: String,
    pub category: Category,
    pub zone: Zone,
    pub phase: Phase,
    pub passed: bool,
    /// The reply (or file text / commit subject) the matcher saw, capped.
    pub observed: String,
    /// Tool calls the model made while answering. A "from memory" probe that used tools may have
    /// re-read the answer off disk, so it is reported next to the score.
    pub tool_calls: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(super) struct Tally {
    pub passed: usize,
    pub total: usize,
}

impl Tally {
    pub(super) fn pct(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            100.0 * self.passed as f64 / self.total as f64
        }
    }

    fn add(&mut self, passed: bool) {
        self.total += 1;
        self.passed += usize::from(passed);
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct PhaseScore {
    pub total: Tally,
    pub by_category: BTreeMap<Category, Tally>,
    pub by_zone: BTreeMap<Zone, Tally>,
    /// Question probes that called a tool anyway (their score may be inflated).
    pub tool_assisted: usize,
}

pub(super) fn score_phase(results: &[ProbeResult], phase: Phase) -> PhaseScore {
    let mut score = PhaseScore::default();
    for r in results.iter().filter(|r| r.phase == phase) {
        score.total.add(r.passed);
        score
            .by_category
            .entry(r.category)
            .or_default()
            .add(r.passed);
        score.by_zone.entry(r.zone).or_default().add(r.passed);
        if r.tool_calls > 0 && r.category != Category::Behavior {
            score.tool_assisted += 1;
        }
    }
    score
}

pub(super) fn cap(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}...")
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub(super) struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub reasoning_tokens: u64,
    pub model_calls: u64,
    pub tool_calls: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct CompactionRecord {
    pub label: &'static str,
    pub messages_before: usize,
    pub messages_after: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One compaction the session itself reported (manual or automatic), as seen through its events.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct CompactionEvent {
    pub auto: bool,
    pub messages_before: usize,
    pub messages_after: usize,
    /// The model that wrote the summary, from the session's "summary via <model>" note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_model: Option<String>,
}

/// Every compaction event of a run, plus the summaries the session rejected as too thin.
#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct CompactionLog {
    pub events: Vec<CompactionEvent>,
    pub thin_summary_warnings: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct CompactionStats {
    pub auto: usize,
    pub manual: usize,
    /// Compactions that fired while the probes ran (they change what later probes see).
    pub during_probes: usize,
    pub thin_summary_warnings: usize,
    pub summary_models: BTreeMap<String, usize>,
}

impl CompactionStats {
    /// `events` is the whole log; the first `scenario_events` of them happened before the probes.
    pub(super) fn from_log(log: &CompactionLog, scenario_events: usize) -> Self {
        let mut stats = Self {
            thin_summary_warnings: log.thin_summary_warnings,
            during_probes: log.events.len().saturating_sub(scenario_events),
            ..Self::default()
        };
        for event in log.events.iter().take(scenario_events) {
            if event.auto {
                stats.auto += 1;
            } else {
                stats.manual += 1;
            }
        }
        for event in &log.events {
            let model = event.summary_model.as_deref().unwrap_or("unknown");
            *stats.summary_models.entry(model.to_string()).or_default() += 1;
        }
        stats
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Report {
    pub scenario: String,
    /// The auto-compaction ceiling the run used, in tokens; absent when the model's own applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compact_cap: Option<u64>,
    pub model: String,
    pub turns_scale: f64,
    pub scenario_turns: usize,
    pub failed_turns: usize,
    pub compactions: Vec<CompactionRecord>,
    pub compaction_stats: CompactionStats,
    pub compaction_events: Vec<CompactionEvent>,
    pub resumed: bool,
    pub after_compaction: PhaseScore,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_resume: Option<PhaseScore>,
    pub usage: UsageTotals,
    pub wall_secs: f64,
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    pub probes: Vec<ProbeResult>,
}

#[cfg(test)]
mod tests {
    use super::super::scenario::{Check, HARD, STANDARD};
    use super::*;

    fn result(id: &str, category: Category, phase: Phase, passed: bool, tools: u64) -> ProbeResult {
        ProbeResult {
            id: id.into(),
            category,
            zone: Zone::Plant,
            phase,
            passed,
            observed: String::new(),
            tool_calls: tools,
            error: None,
        }
    }

    #[test]
    fn a_matcher_needs_every_all_pattern_and_no_none_pattern() {
        let m = Matcher {
            all: &["port\\s+47123", "staging"],
            none: &["8080"],
        };
        assert!(m.accepts("The STAGING server is on port 47123."));
        assert!(
            !m.accepts("staging port 47123 not 8080"),
            "a none pattern vetoes"
        );
        assert!(
            !m.accepts("staging is on port 1"),
            "a missing all pattern fails"
        );
        assert!(
            !Matcher {
                all: &["("],
                none: &[]
            }
            .accepts("("),
            "an invalid regex never passes"
        );
    }

    #[test]
    fn anchors_apply_per_line() {
        let m = Matcher {
            all: &["^JIRA-123"],
            none: &[],
        };
        assert!(m.accepts("JIRA-123: fix"));
        assert!(!m.accepts("fix JIRA-123: late"));
    }

    #[test]
    fn scoring_rolls_up_per_category_and_phase_and_counts_tool_assisted_answers() {
        let results = vec![
            result("a", Category::Number, Phase::AfterCompaction, true, 0),
            result("b", Category::Number, Phase::AfterCompaction, false, 2),
            result("c", Category::Pending, Phase::AfterCompaction, true, 1),
            result("d", Category::Behavior, Phase::AfterCompaction, true, 5),
            result("a", Category::Number, Phase::AfterResume, false, 0),
        ];
        let s = score_phase(&results, Phase::AfterCompaction);
        assert_eq!(
            s.total,
            Tally {
                passed: 3,
                total: 4
            }
        );
        assert_eq!(
            s.by_category[&Category::Number],
            Tally {
                passed: 1,
                total: 2
            }
        );
        assert_eq!(
            s.tool_assisted, 2,
            "behavior probes are expected to use tools"
        );
        let r = score_phase(&results, Phase::AfterResume);
        assert_eq!(
            r.total,
            Tally {
                passed: 0,
                total: 1
            }
        );
        assert!((s.total.pct() - 75.0).abs() < 1e-9);
        assert_eq!(Tally::default().pct(), 0.0);
    }

    #[test]
    fn cap_truncates_on_char_boundaries() {
        assert_eq!(cap("  short  ", 10), "short");
        assert_eq!(cap("ééééé", 3), "ééé...");
    }

    #[test]
    fn every_probe_matcher_compiles_and_separates_its_good_and_bad_sample() {
        for scenario in [&STANDARD, &HARD] {
            for p in scenario.probes() {
                let matchers: Vec<Matcher> = match p.check {
                    Check::Answer(m) | Check::CommitSubject(m) => vec![m],
                    Check::File { matcher, .. } => vec![matcher],
                    Check::WorkState { pending } => {
                        // The file half is checked against git in the driver tests; the pending
                        // half is a plain matcher list that the good sample satisfies.
                        assert!(pending.iter().all(|m| m.patterns_compile()), "{}", p.id);
                        assert!(pending.iter().all(|m| m.accepts(p.good)), "{}: good", p.id);
                        assert!(!pending.iter().all(|m| m.accepts(p.bad)), "{}: bad", p.id);
                        continue;
                    }
                };
                for matcher in matchers {
                    assert!(
                        matcher.patterns_compile(),
                        "{}: pattern does not compile",
                        p.id
                    );
                    assert!(matcher.accepts(p.good), "{}: good sample rejected", p.id);
                    assert!(!matcher.accepts(p.bad), "{}: bad sample accepted", p.id);
                }
            }
        }
    }

    #[test]
    fn compaction_stats_split_auto_from_manual_and_count_summary_models() {
        let event = |auto, model: Option<&str>| CompactionEvent {
            auto,
            messages_before: 30,
            messages_after: 8,
            summary_model: model.map(String::from),
        };
        let log = CompactionLog {
            events: vec![
                event(true, Some("m1")),
                event(true, Some("m1")),
                event(false, Some("m2")),
                event(true, None),
            ],
            thin_summary_warnings: 1,
        };
        let stats = CompactionStats::from_log(&log, 3);
        assert_eq!((stats.auto, stats.manual, stats.during_probes), (2, 1, 1));
        assert_eq!(stats.summary_models["m1"], 2);
        assert_eq!(stats.summary_models["unknown"], 1);
        assert_eq!(stats.thin_summary_warnings, 1);
    }
}
