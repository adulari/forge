//! Missions (docs/features/goal-contract.md, "Missions (milestones)"): a big goal as an ordered
//! list of milestones, each with its own assertions. A mission is still one [`GoalContract`]:
//! the assertions of every milestone are numbered once across the whole contract, and the
//! `mission` field says which of them belong to which milestone and which milestone is current.
//! Only the assertions of the current and earlier milestones are active; the validator runs those
//! (so a regression in an earlier milestone shows up at every check, boundaries included), and
//! the mission is achieved when the last milestone is current and everything passes.

use serde::{Deserialize, Serialize};

use super::{assertion_from, clip, failure_line, first_of, Assertion, GoalContract, TEXT_CHARS};

pub const MIN_MILESTONES: usize = 2;
/// The most milestones a mission can have whatever `[goal] mission_max_milestones` says.
pub const MAX_MILESTONES_HARD: usize = 12;
pub const DEFAULT_MAX_MILESTONES: usize = 6;
pub const MIN_MILESTONE_ASSERTIONS: usize = 2;
pub const MAX_MILESTONE_ASSERTIONS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
    /// The contract assertions that prove this milestone.
    pub assertion_ids: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mission {
    pub milestones: Vec<Milestone>,
    /// 1-based number of the milestone being worked on.
    pub current: u32,
}

impl Mission {
    pub fn total(&self) -> usize {
        self.milestones.len()
    }

    pub fn is_last(&self) -> bool {
        self.current as usize >= self.milestones.len()
    }

    /// 1-based number of the milestone `assertion_id` belongs to.
    pub fn milestone_of(&self, assertion_id: u32) -> Option<u32> {
        self.milestones
            .iter()
            .position(|m| m.assertion_ids.contains(&assertion_id))
            .map(|i| i as u32 + 1)
    }

    pub fn current_milestone(&self) -> Option<&Milestone> {
        self.milestones.get((self.current as usize).checked_sub(1)?)
    }
}

/// How a milestone stands on the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilestoneState {
    Done,
    Active,
    Upcoming,
}

impl MilestoneState {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Active => "active",
            Self::Upcoming => "upcoming",
        }
    }
}

impl GoalContract {
    /// A contract that is a mission with at least one milestone.
    pub fn mission_ref(&self) -> Option<&Mission> {
        self.mission.as_ref().filter(|m| !m.milestones.is_empty())
    }

    /// Whether the validator runs this assertion now: always for a plain contract, and for a
    /// mission when its milestone is the current one or an earlier one.
    pub fn is_active(&self, id: u32) -> bool {
        match self.mission_ref() {
            None => true,
            Some(m) => m.milestone_of(id).is_some_and(|n| n <= m.current),
        }
    }

    pub fn active_assertions(&self) -> impl Iterator<Item = &Assertion> {
        self.assertions.iter().filter(|a| self.is_active(a.id))
    }

    /// The contract the validator runs: only the active assertions, with no mission and no
    /// results of its own.
    pub fn validation_scope(&self) -> GoalContract {
        let mut scope = GoalContract::new(self.active_assertions().cloned().collect());
        scope.approved = self.approved;
        scope
    }

    /// Passing and total assertions of milestone `n` (1-based).
    pub fn milestone_counts(&self, n: u32) -> (usize, usize) {
        let Some(m) = self.mission_ref() else {
            return (self.passed_count(), self.assertions.len());
        };
        let ids = m
            .milestones
            .get(n as usize - 1)
            .map_or(&[][..], |m| m.assertion_ids.as_slice());
        let passed = ids
            .iter()
            .filter(|id| self.result(**id).is_some_and(|r| r.passed))
            .count();
        (passed, ids.len())
    }

    pub fn milestone_state(&self, n: u32) -> MilestoneState {
        match self.mission_ref() {
            Some(m) if n < m.current => MilestoneState::Done,
            Some(m) if n == m.current => MilestoneState::Active,
            Some(_) => MilestoneState::Upcoming,
            None => MilestoneState::Active,
        }
    }

    /// Every active assertion has a passing result: the current milestone and every earlier one.
    pub fn active_all_passed(&self) -> bool {
        let mut any = false;
        for a in self.active_assertions() {
            any = true;
            if !self.result(a.id).is_some_and(|r| r.passed) {
                return false;
            }
        }
        any
    }

    /// Assertions of earlier milestones that do not pass: regressions.
    pub fn regressed_ids(&self) -> Vec<u32> {
        let Some(m) = self.mission_ref() else {
            return Vec::new();
        };
        self.failing_ids()
            .into_iter()
            .filter(|id| m.milestone_of(*id).is_some_and(|n| n < m.current))
            .collect()
    }

    /// Move to the next milestone. `false` when there is none.
    pub fn advance_milestone(&mut self) -> bool {
        match self.mission.as_mut() {
            Some(m) if !m.is_last() => {
                m.current += 1;
                true
            }
            _ => false,
        }
    }

    /// `milestone 2/4: 3/5 pass`, or `milestone 2/4: 5 assertions` before the validator ran.
    pub fn mission_headline(&self) -> Option<String> {
        let m = self.mission_ref()?;
        let (passed, total) = self.milestone_counts(m.current);
        let tally = if self.results.is_empty() {
            format!("{total} assertions")
        } else {
            format!("{passed}/{total} pass")
        };
        Some(format!("milestone {}/{}: {tally}", m.current, m.total()))
    }

    /// The table of a mission: one block per milestone with its assertions under it.
    pub fn mission_table_lines(&self) -> Option<Vec<String>> {
        let m = self.mission_ref()?;
        let mut out = Vec::new();
        for (i, milestone) in m.milestones.iter().enumerate() {
            let n = i as u32 + 1;
            let (passed, total) = self.milestone_counts(n);
            let tag = match self.milestone_state(n) {
                MilestoneState::Done => format!("done {passed}/{total}"),
                MilestoneState::Active => format!("current {passed}/{total} pass"),
                MilestoneState::Upcoming => format!("upcoming, {total} assertions"),
            };
            out.push(format!(
                "milestone {n}/{} - {} [{tag}]",
                m.total(),
                milestone.title
            ));
            for id in &milestone.assertion_ids {
                if let Some(a) = self.assertions.iter().find(|a| a.id == *id) {
                    self.assertion_lines(a, &mut out);
                }
            }
        }
        Some(out)
    }
}

/// What the worker is told for a mission: the current milestone's assertions to build toward, and
/// the earlier ones it must not break.
pub fn worker_guidance(contract: &GoalContract) -> Option<String> {
    let m = contract.mission_ref()?;
    let current = m.current_milestone()?;
    let mut out = format!(
        "This goal is a mission of {} milestones, done in order, each by a fresh worker. You are \
         on milestone {}/{}: {}. An independent validator runs every check itself after each \
         turn; the milestone is done only when all of its checks pass and no earlier milestone's \
         check has broken. Do not claim completion on your own judgement, and do not work ahead \
         on later milestones.\n",
        m.total(),
        m.current,
        m.total(),
        current.title
    );
    for id in &current.assertion_ids {
        if let Some(a) = contract.assertions.iter().find(|a| a.id == *id) {
            out.push_str(&format!("{}. {} - check: {}\n", a.id, a.text, a.command));
        }
    }
    let kept: Vec<String> = contract
        .active_assertions()
        .filter(|a| !current.assertion_ids.contains(&a.id))
        .map(|a| format!("{}. {} - check: {}", a.id, a.text, a.command))
        .collect();
    if !kept.is_empty() {
        out.push_str("Earlier milestones' checks that must keep passing:\n");
        for line in kept {
            out.push_str(&line);
            out.push('\n');
        }
    }
    Some(out)
}

/// The prompt after a validator run with failures in a mission. Regressions come first and are
/// called out: they break what an earlier milestone already proved.
pub fn failure_prompt(goal: &str, contract: &GoalContract) -> Option<String> {
    let m = contract.mission_ref()?;
    let title = m.current_milestone().map_or("", |c| c.title.as_str());
    let active = contract.active_assertions().count();
    let passed = active - contract.failing_ids().len();
    let mut out = format!(
        "Goal: {goal}\nMission milestone {}/{}: {title}\nThe independent validator ran the \
         mission's checks: {passed} of {active} assertions pass.",
        m.current,
        m.total()
    );
    let regressed = contract.regressed_ids();
    if regressed.is_empty() {
        out.push_str(" Fix what fails, then stop; the validator re-runs every check.\n");
    } else {
        out.push_str(
            " An assertion from an earlier milestone no longer passes - a regression. Fix the \
             regression first, then finish this milestone; the validator re-runs every check.\n",
        );
    }
    let mut failing = contract.failing_ids();
    failing.sort_by_key(|id| !regressed.contains(id));
    for id in failing {
        let tag = match m.milestone_of(id) {
            Some(n) if regressed.contains(&id) => format!("REGRESSION (milestone {n}) "),
            _ => String::new(),
        };
        if let Some(line) = failure_line(contract, id) {
            out.push_str(&format!("{tag}{line}"));
        }
    }
    Some(out)
}

/// The mission lines of a fresh worker's brief: where the mission stands and what comes later.
pub fn brief_lines(contract: &GoalContract) -> Option<String> {
    let m = contract.mission_ref()?;
    let current = m.current_milestone()?;
    let mut out = format!(
        "Mission: you are on milestone {}/{} - {}. Milestones before it passed and their checks \
         are re-run to catch regressions.\n",
        m.current,
        m.total(),
        current.title
    );
    let later: Vec<String> = m
        .milestones
        .iter()
        .enumerate()
        .skip(m.current as usize)
        .map(|(i, ms)| format!("{}. {}", i + 1, ms.title))
        .collect();
    if !later.is_empty() {
        out.push_str(&format!(
            "Later milestones (not yours yet): {}\n",
            later.join("; ")
        ));
    }
    Some(out)
}

/// The first prompt of a milestone that starts in the same session (no fresh worker was
/// available): what passed and what the next milestone must prove.
pub fn milestone_prompt(goal: &str, contract: &GoalContract) -> Option<String> {
    let m = contract.mission_ref()?;
    let current = m.current_milestone()?;
    let mut out = format!(
        "Goal: {goal}\nMilestone {} passed. Now milestone {}/{}: {}\nIts checks, which the \
         independent validator runs after each turn:\n",
        m.current - 1,
        m.current,
        m.total(),
        current.title
    );
    for id in &current.assertion_ids {
        if let Some(a) = contract.assertions.iter().find(|a| a.id == *id) {
            out.push_str(&format!("{}. {} - check: {}\n", a.id, a.text, a.command));
        }
    }
    out.push_str("Earlier milestones' checks must keep passing. Do not work ahead.");
    Some(out)
}

/// One milestone of a model-written draft, before the assertions are numbered across the mission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftMilestone {
    pub title: String,
    pub assertions: Vec<Assertion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionDraft {
    pub milestones: Vec<DraftMilestone>,
}

fn milestone_list(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(items) => Some(items),
        serde_json::Value::Object(map) => {
            first_of(map, &["milestones", "stages", "phases"]).and_then(|v| v.as_array())
        }
        _ => None,
    }
}

fn draft_milestone(value: &serde_json::Value) -> Option<DraftMilestone> {
    let map = value.as_object()?;
    let title = super::text_of(first_of(
        map,
        &[
            "title",
            "name",
            "milestone",
            "goal",
            "summary",
            "description",
        ],
    ));
    let items = first_of(map, &["assertions", "checks", "criteria"])?.as_array()?;
    if title.is_empty() {
        return None;
    }
    let mut assertions: Vec<Assertion> = Vec::new();
    for item in items {
        if let Some(a) = assertion_from(item, assertions.len() as u32 + 1) {
            assertions.push(a);
        }
        if assertions.len() == MAX_MILESTONE_ASSERTIONS {
            break;
        }
    }
    Some(DraftMilestone {
        title: clip(&title, TEXT_CHARS),
        assertions,
    })
}

impl MissionDraft {
    /// Keep only the assertions `keep` accepts; milestones left with too few are dropped.
    pub fn retain(&mut self, keep: impl Fn(&Assertion) -> bool) {
        for m in &mut self.milestones {
            m.assertions.retain(&keep);
        }
        self.milestones
            .retain(|m| m.assertions.len() >= MIN_MILESTONE_ASSERTIONS);
    }

    /// Cut to `max` milestones, in order.
    pub fn cap(&mut self, max: usize) {
        self.milestones
            .truncate(max.clamp(MIN_MILESTONES, MAX_MILESTONES_HARD));
    }

    pub fn is_usable(&self) -> bool {
        self.milestones.len() >= MIN_MILESTONES
            && self
                .milestones
                .iter()
                .all(|m| m.assertions.len() >= MIN_MILESTONE_ASSERTIONS)
    }

    /// The milestones as one contract: assertions numbered 1.. across the mission, milestone 1
    /// current. `None` when the draft is not usable.
    pub fn into_contract(self) -> Option<GoalContract> {
        if !self.is_usable() {
            return None;
        }
        let mut assertions: Vec<Assertion> = Vec::new();
        let mut milestones: Vec<Milestone> = Vec::new();
        for m in self.milestones {
            let mut ids = Vec::new();
            for mut a in m.assertions {
                a.id = assertions.len() as u32 + 1;
                ids.push(a.id);
                assertions.push(a);
            }
            milestones.push(Milestone {
                title: m.title,
                assertion_ids: ids,
            });
        }
        let mut contract = GoalContract::new(assertions);
        contract.mission = Some(Mission {
            milestones,
            current: 1,
        });
        Some(contract)
    }

    /// The card shown for approval: one step per milestone listing its checks.
    pub fn card_steps(&self) -> Vec<(String, String)> {
        self.milestones
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let detail = m
                    .assertions
                    .iter()
                    .map(|a| format!("- {} (check: {})", a.text, a.command))
                    .collect::<Vec<_>>()
                    .join("\n");
                (format!("Milestone {}: {}", i + 1, m.title), detail)
            })
            .collect()
    }
}

/// Parse a model's mission reply: the first JSON object with a `milestones` array (or a bare
/// array of milestones), tolerant of fences and prose. A milestone needs a title and
/// [`MIN_MILESTONE_ASSERTIONS`] usable assertions or it is dropped; at most `max` milestones are
/// kept. `None` when fewer than [`MIN_MILESTONES`] remain.
pub fn parse_mission(text: &str, max: usize) -> Option<MissionDraft> {
    let items = text
        .char_indices()
        .filter(|(_, c)| matches!(c, '{' | '['))
        .find_map(|(i, _)| {
            let mut stream =
                serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
            match stream.next() {
                Some(Ok(value)) => milestone_list(&value).cloned(),
                _ => None,
            }
        })?;
    let mut draft = MissionDraft {
        milestones: items.iter().filter_map(draft_milestone).collect(),
    };
    draft
        .milestones
        .retain(|m| m.assertions.len() >= MIN_MILESTONE_ASSERTIONS);
    draft.cap(max);
    draft.is_usable().then_some(draft)
}

#[cfg(test)]
#[path = "mission_tests.rs"]
mod tests;
