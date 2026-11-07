//! Fresh-worker relay (docs/features/goal-contract.md, "Fresh-worker relay"): a `/goal` that
//! wedged, or whose session is worn out, moves to a new session on the same worktree with a
//! deterministic brief of what the old one produced. Nothing here calls a model: the brief is
//! read from git, the contract, the task list, the compaction index and the stored turns.

use forge_types::goal_contract::GoalContract;
use forge_types::TodoStatus;

use crate::{CoreError, Session};

/// Characters of a failing assertion's proof the brief carries.
const PROOF_TAIL: usize = 300;
/// Open tasks listed in the brief.
const MAX_TASKS: usize = 12;

/// Why a goal moves to a fresh worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayReason {
    /// The evaluator or the contract validator stopped seeing movement.
    Wedged,
    /// Turns without task progress ran out the stall counter.
    Stalled,
    /// The session was compacted `[goal] relay_after_compactions` times.
    Worn,
    /// A mission milestone passed: the next one starts in a fresh worker. Not a failure, so it
    /// does not count against `max_relays`.
    Milestone,
}

impl RelayReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wedged => "wedged",
            Self::Stalled => "stalled",
            Self::Worn => "worn",
            Self::Milestone => "milestone",
        }
    }

    pub fn parse(tag: &str) -> Option<Self> {
        Some(match tag {
            "wedged" => Self::Wedged,
            "stalled" => Self::Stalled,
            "worn" => Self::Worn,
            "milestone" => Self::Milestone,
            _ => return None,
        })
    }

    /// The reason a goal's stop message belongs to, when that stop can be handed to a fresh
    /// worker. A completion, a budget, a decline or an interruption never can.
    pub fn of_stop(stop: &str) -> Option<Self> {
        if stop.contains("goal wedged") || stop.contains("the agent reports done but the evaluator")
        {
            Some(Self::Wedged)
        } else if stop.contains("goal stalled") {
            Some(Self::Stalled)
        } else {
            None
        }
    }

    /// The line a surface shows: `S1 handed off (wedged)`.
    pub fn label(self) -> &'static str {
        self.as_str()
    }
}

/// Whether the goal may be handed on again: relays are on and the cap is not reached.
pub fn relay_allowed(max_relays: u32, relays_so_far: u32) -> bool {
    max_relays > 0 && relays_so_far < max_relays
}

/// Whether the session has been compacted often enough to be worn out. `0` turns it off.
pub fn is_worn(after_compactions: u32, compactions: u32) -> bool {
    after_compactions > 0 && compactions >= after_compactions
}

/// What decides whether a goal moves to a fresh worker now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayPolicy {
    pub max_relays: u32,
    pub after_compactions: u32,
    /// Hand-offs that led to the session now running the goal.
    pub depth: u32,
    pub compactions: u32,
}

impl RelayPolicy {
    pub fn allowed(&self) -> bool {
        relay_allowed(self.max_relays, self.depth)
    }

    pub fn worn(&self) -> bool {
        is_worn(self.after_compactions, self.compactions)
    }
}

/// Everything the brief says, already gathered.
#[derive(Debug, Clone, Default)]
pub struct BriefFacts {
    pub goal: String,
    pub reason: String,
    pub detail: String,
    pub predecessor_id: String,
    /// Hand-offs this goal has made, this one included.
    pub relay_no: u32,
    pub max_relays: u32,
    pub end_reason: String,
    pub commits: String,
    pub uncommitted: String,
    pub contract: Option<GoalContract>,
    pub open_tasks: Vec<String>,
    pub episodes: String,
    pub expert: Option<String>,
    pub envelope: Option<String>,
    pub tokens_used: u64,
    pub iteration: usize,
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.trim().to_string();
    }
    let kept: String = text.chars().skip(count - max).collect();
    format!("…{}", kept.trim())
}

fn or_none(text: &str) -> String {
    if text.trim().is_empty() {
        "(none)".to_string()
    } else {
        text.trim().to_string()
    }
}

/// The brief the successor's first turn carries. Same facts, same text.
pub fn render_brief(f: &BriefFacts) -> String {
    let short = &f.predecessor_id[..f.predecessor_id.len().min(8)];
    let mut out = format!(
        "Goal: {}\n\nYou are a fresh worker taking over this goal from session {short} \
         (hand-off {}{}). It was handed on because it {}{}. The work it did is already in \
         this workspace: read it, do not redo it.\n",
        f.goal,
        f.relay_no,
        if f.reason == "milestone" {
            String::new()
        } else {
            format!(" of {}", f.max_relays)
        },
        match f.reason.as_str() {
            "wedged" => "stopped making progress",
            "stalled" => "stalled without task progress",
            "worn" => "was worn out by repeated compaction",
            "milestone" => "finished its milestone",
            other => other,
        },
        if f.detail.trim().is_empty() {
            String::new()
        } else {
            format!(" ({})", f.detail.trim())
        }
    );
    out.push_str(&format!(
        "Its last turn ended: {}. It ran {} iterations and {} tokens.\n",
        or_none(&f.end_reason),
        f.iteration,
        f.tokens_used
    ));
    out.push_str(&format!(
        "\nCommits since the goal started:\n{}\n\nUncommitted files:\n{}\n",
        or_none(&f.commits),
        or_none(&f.uncommitted)
    ));
    if let Some(contract) = &f.contract {
        out.push_str(&format!("\nGoal contract: {}\n", contract.headline()));
        if let Some(mission) = forge_types::goal_contract::mission::brief_lines(contract) {
            out.push_str(&mission);
        }
        for a in contract.active_assertions() {
            match contract.result(a.id) {
                Some(r) if r.passed => {
                    out.push_str(&format!("PASS {}. {}\n", a.id, a.text));
                }
                Some(r) => {
                    out.push_str(&format!(
                        "FAIL {}. {}\n  check: {}\n",
                        a.id, a.text, a.command
                    ));
                    if !r.reason.is_empty() {
                        out.push_str(&format!("  why: {}\n", r.reason));
                    }
                    if !r.proof.trim().is_empty() {
                        out.push_str(&format!(
                            "  proof: {}\n",
                            tail(&r.proof, PROOF_TAIL).replace('\n', "\n    ")
                        ));
                    }
                }
                None => out.push_str(&format!("NOT RUN {}. {}\n", a.id, a.text)),
            }
        }
        out.push_str("The independent validator re-runs every check after your turns.\n");
    }
    out.push_str("\nOpen tasks:\n");
    if f.open_tasks.is_empty() {
        out.push_str("(none recorded: write a plan with update_tasks)\n");
    }
    for task in f.open_tasks.iter().take(MAX_TASKS) {
        out.push_str(&format!("- {task}\n"));
    }
    if f.open_tasks.len() > MAX_TASKS {
        out.push_str(&format!(
            "- … and {} more\n",
            f.open_tasks.len() - MAX_TASKS
        ));
    }
    if !f.episodes.trim().is_empty() {
        out.push_str(&format!(
            "\nWhat the old session covered (its message seq ranges):\n{}\n",
            f.episodes.trim()
        ));
    }
    if let Some(expert) = f.expert.as_deref().filter(|e| !e.trim().is_empty()) {
        out.push_str(&format!(
            "\nThe expert on call said: {} It can be wrong; check it against the files.\n",
            expert.trim()
        ));
    }
    if let Some(envelope) = f.envelope.as_deref().filter(|e| !e.is_empty()) {
        out.push_str(&format!(
            "\nSpend envelope (shared with the old session): {envelope}\n"
        ));
    }
    out.push_str(
        "\nThe `recall` tool searches the old session's history: use it for a decision or error \
         the brief does not carry. Do not repeat what already works. Take the single most \
         valuable open step, finish it end to end, verify it, commit finished work, and keep \
         update_tasks current.",
    );
    out
}

/// What a relay needs from the goal run.
#[derive(Debug, Clone)]
pub struct RelayRequest {
    pub goal: String,
    pub started_unix: i64,
    pub reason: RelayReason,
    pub detail: String,
    pub contract: Option<GoalContract>,
    pub max_relays: u32,
    pub envelope: Option<String>,
    pub tokens_used: u64,
    pub iteration: usize,
}

/// The fresh session and the brief it starts from.
#[derive(Debug, Clone)]
pub struct RelayHandOff {
    pub successor_id: String,
    pub brief: String,
}

/// The "Earlier episodes" section of a compaction summary, or empty.
fn episode_section(summary: &str) -> String {
    let Some(at) = summary.find(crate::compaction_episodes::EPISODES_HEADING) else {
        return String::new();
    };
    let rest = &summary[at + crate::compaction_episodes::EPISODES_HEADING.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter(|l| l.starts_with("- seq "))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Session {
    /// Times this session has been compacted.
    pub fn compaction_count(&self) -> u32 {
        self.store.compaction_count(&self.id).unwrap_or(0)
    }

    /// Failure hand-offs that led to this session; a mission's milestone hand-offs do not count.
    pub fn relay_depth(&self) -> u32 {
        self.store
            .relay_predecessors(&self.id)
            .unwrap_or_default()
            .iter()
            .filter(|p| {
                self.store
                    .relay_successor(p)
                    .ok()
                    .flatten()
                    .is_none_or(|link| link.reason != RelayReason::Milestone.as_str())
            })
            .count() as u32
    }

    /// The relay settings and where this session stands against them.
    pub fn relay_policy(&self) -> RelayPolicy {
        RelayPolicy {
            max_relays: self.config.goal.max_relays,
            after_compactions: self.config.goal.relay_after_compactions,
            depth: self.relay_depth(),
            compactions: self.compaction_count(),
        }
    }

    /// Gather the facts a successor starts from.
    pub async fn relay_facts(&mut self, request: &RelayRequest) -> BriefFacts {
        let evidence =
            crate::goal_evidence::collect(self.workspace.root(), &self.id, request.started_unix)
                .await;
        let episodes = self
            .store
            .session_compaction_summary(&self.id)
            .ok()
            .flatten()
            .map(|(summary, _)| episode_section(&summary))
            .unwrap_or_default();
        let expert = self
            .store
            .recent_expert_turns(0, Some(&self.id), 5)
            .unwrap_or_default()
            .into_iter()
            .find_map(|row| {
                let detail: serde_json::Value =
                    serde_json::from_str(row.detail_json.as_deref()?).ok()?;
                let diagnosis = detail["expert_diagnosis"].as_str()?.to_string();
                let model = detail["expert_model"].as_str().unwrap_or("expert");
                Some(format!("({model}) {diagnosis}"))
            });
        let tasks = self.store.tasks(&self.id).unwrap_or_default();
        let tasks = if tasks.is_empty() {
            self.tasks().to_vec()
        } else {
            tasks
        };
        BriefFacts {
            goal: request.goal.clone(),
            reason: request.reason.as_str().to_string(),
            detail: request.detail.clone(),
            predecessor_id: self.id.clone(),
            relay_no: self.relay_depth() + 1,
            max_relays: request.max_relays,
            end_reason: self.last_end_reason().unwrap_or_default(),
            commits: evidence.commits,
            uncommitted: evidence.uncommitted,
            contract: request.contract.clone(),
            open_tasks: tasks
                .iter()
                .filter(|t| t.status != TodoStatus::Done)
                .map(|t| format!("[{:?}] {}", t.status, t.title).to_lowercase())
                .collect(),
            episodes,
            expert,
            envelope: request.envelope.clone(),
            tokens_used: request.tokens_used,
            iteration: request.iteration,
        }
    }

    /// Create the successor session for this goal and mark this session's last turn
    /// `handed_off`. The caller starts the successor's driver.
    pub async fn hand_off_goal(
        &mut self,
        request: &RelayRequest,
    ) -> Result<RelayHandOff, CoreError> {
        let facts = self.relay_facts(request).await;
        let brief = render_brief(&facts);
        let successor_id = self.store.create_relay_successor(
            &self.id,
            request.reason.as_str(),
            &request.detail,
            &brief,
        )?;
        Ok(RelayHandOff {
            successor_id,
            brief,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::goal_contract::{Assertion, AssertionResult};

    fn contract() -> GoalContract {
        let mut c = GoalContract::new(vec![
            Assertion {
                id: 1,
                text: "notes exist".into(),
                command: "test -f notes.txt".into(),
                expect: String::new(),
                contains: None,
            },
            Assertion {
                id: 2,
                text: "notes say ready".into(),
                command: "cat notes.txt".into(),
                expect: String::new(),
                contains: Some("ready".into()),
            },
        ]);
        c.approved = true;
        c.record(
            vec![
                AssertionResult {
                    id: 1,
                    passed: true,
                    proof: String::new(),
                    reason: String::new(),
                },
                AssertionResult {
                    id: 2,
                    passed: false,
                    proof: "draft\nnot yet".into(),
                    reason: "missing ready".into(),
                },
            ],
            100,
        );
        c
    }

    fn facts() -> BriefFacts {
        BriefFacts {
            goal: "ship the parser".into(),
            reason: "wedged".into(),
            detail: "assertion 2 still fails".into(),
            predecessor_id: "0123456789abcdef".into(),
            relay_no: 1,
            max_relays: 3,
            end_reason: "final_answer".into(),
            commits: "abc123 feat: lexer".into(),
            uncommitted: "M src/parser.rs".into(),
            contract: Some(contract()),
            open_tasks: vec!["[pending] wire the cli".into()],
            episodes: "- seq 0-40: write the lexer".into(),
            expert: Some("(codex::sol) the tokenizer loops".into()),
            envelope: Some("$3.20 of $10".into()),
            tokens_used: 5000,
            iteration: 9,
        }
    }

    #[test]
    fn the_brief_carries_every_fact_and_is_deterministic() {
        let brief = render_brief(&facts());
        for needle in [
            "Goal: ship the parser",
            "session 01234567",
            "hand-off 1 of 3",
            "stopped making progress (assertion 2 still fails)",
            "abc123 feat: lexer",
            "M src/parser.rs",
            "PASS 1. notes exist",
            "FAIL 2. notes say ready",
            "check: cat notes.txt",
            "why: missing ready",
            "not yet",
            "[pending] wire the cli",
            "- seq 0-40: write the lexer",
            "The expert on call said: (codex::sol) the tokenizer loops",
            "$3.20 of $10",
            "final_answer",
            "`recall`",
        ] {
            assert!(brief.contains(needle), "missing {needle:?} in\n{brief}");
        }
        assert_eq!(brief, render_brief(&facts()));
    }

    #[test]
    fn a_brief_without_expert_or_contract_says_so_plainly() {
        let mut f = facts();
        f.expert = None;
        f.contract = None;
        f.open_tasks.clear();
        f.commits.clear();
        let brief = render_brief(&f);
        assert!(!brief.contains("expert on call"));
        assert!(!brief.contains("Goal contract"));
        assert!(brief.contains("(none recorded: write a plan"));
        assert!(brief.contains("Commits since the goal started:\n(none)"));
    }

    #[test]
    fn only_wedges_and_stalls_can_be_handed_on() {
        assert_eq!(
            RelayReason::of_stop("🎯 goal wedged — no progress across 3 evaluations: x"),
            Some(RelayReason::Wedged)
        );
        assert_eq!(
            RelayReason::of_stop("🎯 goal wedged — assertions 2 still fail after 3 validations"),
            Some(RelayReason::Wedged)
        );
        assert_eq!(
            RelayReason::of_stop("🎯 goal stalled — no task progress, stopping"),
            Some(RelayReason::Stalled)
        );
        assert_eq!(
            RelayReason::of_stop(
                "🎯 goal stopped — the agent reports done but the evaluator disagrees: r"
            ),
            Some(RelayReason::Wedged)
        );
        for stop in [
            "🎯 goal complete",
            "🎯 goal stopped — budget cap reached",
            "🎯 goal stopped — iteration ceiling",
            "🎯 goal not started — the contract was declined",
        ] {
            assert_eq!(RelayReason::of_stop(stop), None, "{stop}");
        }
    }

    #[test]
    fn the_cap_and_the_wear_threshold() {
        assert!(relay_allowed(3, 0) && relay_allowed(3, 2));
        assert!(!relay_allowed(3, 3));
        assert!(!relay_allowed(0, 0));
        assert!(is_worn(4, 4) && is_worn(4, 9));
        assert!(!is_worn(4, 3));
        assert!(!is_worn(0, 50));
    }

    #[test]
    fn the_episode_index_is_cut_out_of_a_compaction_summary() {
        let summary = format!(
            "1. Primary request\nship\n\n{}\nintro line\n- seq 0-9: lexer\n- seq 10-20: parser\n\n## Next\n- seq 99-100: no",
            crate::compaction_episodes::EPISODES_HEADING
        );
        assert_eq!(
            episode_section(&summary),
            "- seq 0-9: lexer\n- seq 10-20: parser"
        );
        assert_eq!(episode_section("nothing here"), "");
    }
}
