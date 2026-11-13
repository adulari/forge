//! The validation contract of a `/goal --contract` run (docs/features/goal-contract.md): the
//! numbered assertions that must be true before the goal counts as done, each with a concrete
//! read-only check, plus the independent validator's per-assertion results.
//!
//! Pure data, parsing and text rendering. Drafting the contract and running the checks need a
//! session and live in forge-core; this module is what every surface (TUI, CLI, snapshot, phone)
//! shares.

use serde::{Deserialize, Serialize};

pub mod behaviour;
pub mod mission;

pub const MIN_ASSERTIONS: usize = 3;
pub const MAX_ASSERTIONS: usize = 12;
pub(crate) const TEXT_CHARS: usize = 200;
pub(crate) const COMMAND_CHARS: usize = 400;
/// How much of a check's output is kept as proof and fed back to the worker.
pub const PROOF_CHARS: usize = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assertion {
    /// 1-based, the number the worker is told about ("assertion 4 failed").
    pub id: u32,
    /// What must be true.
    pub text: String,
    /// The read-only shell command that proves it.
    pub command: String,
    /// The expected outcome in words ("exit 0", "prints 3"); shown on the card.
    #[serde(default)]
    pub expect: String,
    /// When set the output must also contain this text; the exit code must be 0 either way.
    #[serde(default)]
    pub contains: Option<String>,
    /// Set for a behaviour assertion: the QA runner starts the thing and probes it, and `command`
    /// holds the one-line summary. Absent for a read-only check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behaviour: Option<behaviour::Behaviour>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionResult {
    pub id: u32,
    pub passed: bool,
    /// The tail of the command's output, or why it was not run.
    pub proof: String,
    /// One line on why it failed; empty when it passed.
    #[serde(default)]
    pub reason: String,
    /// A screenshot a behaviour probe took, stored in the session's evidence folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

impl AssertionResult {
    /// A behaviour probe that could not run (no browser tool, say). It counts as passed so a
    /// missing tool cannot wedge the goal, but every surface lists it apart from a real pass.
    pub fn unverified(&self) -> bool {
        self.passed && self.reason.starts_with(UNVERIFIED)
    }
}

pub const UNVERIFIED: &str = "unverified:";

/// A contract and what the validator has found so far. `approved` is false only while the card
/// waits for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalContract {
    pub assertions: Vec<Assertion>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub results: Vec<AssertionResult>,
    /// How many times the validator has run.
    #[serde(default)]
    pub rounds: u32,
    /// Unix seconds of the last validator run.
    #[serde(default)]
    pub checked_at: Option<i64>,
    /// Set when the goal is a mission: which assertions belong to which milestone and which
    /// milestone is current (docs/features/goal-contract.md, "Missions (milestones)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<mission::Mission>,
}

impl GoalContract {
    pub fn new(assertions: Vec<Assertion>) -> Self {
        Self {
            assertions,
            approved: false,
            results: Vec::new(),
            rounds: 0,
            checked_at: None,
            mission: None,
        }
    }

    /// The validator's latest verdict: every assertion has a passing result.
    pub fn all_passed(&self) -> bool {
        !self.assertions.is_empty()
            && self
                .assertions
                .iter()
                .all(|a| self.result(a.id).is_some_and(|r| r.passed))
    }

    pub fn result(&self, id: u32) -> Option<&AssertionResult> {
        self.results.iter().find(|r| r.id == id)
    }

    pub fn passed_count(&self) -> usize {
        self.assertions
            .iter()
            .filter(|a| self.result(a.id).is_some_and(|r| r.passed))
            .count()
    }

    /// Active assertions without a passing result (all of them outside a mission).
    pub fn failing_ids(&self) -> Vec<u32> {
        self.active_assertions()
            .filter(|a| !self.result(a.id).is_some_and(|r| r.passed))
            .map(|a| a.id)
            .collect()
    }

    /// Ids of passed assertions that were not actually verified.
    pub fn unverified_ids(&self) -> Vec<u32> {
        self.results
            .iter()
            .filter(|r| r.unverified())
            .map(|r| r.id)
            .collect()
    }

    /// `3/6 pass`, or `6 assertions` before the validator has run.
    pub fn headline(&self) -> String {
        if let Some(line) = self.mission_headline() {
            return line;
        }
        if self.results.is_empty() {
            format!("{} assertions", self.assertions.len())
        } else {
            let unverified = self.unverified_ids().len();
            let tail = if unverified > 0 {
                format!(" ({unverified} unverified)")
            } else {
                String::new()
            };
            format!(
                "{}/{} pass{tail}",
                self.passed_count(),
                self.assertions.len()
            )
        }
    }

    /// Fold in a new validator run. A mission validates only its active assertions, so the results
    /// of the rest are kept.
    pub fn record(&mut self, results: Vec<AssertionResult>, now: i64) {
        if self.mission.is_some() {
            self.results
                .retain(|old| !results.iter().any(|new| new.id == old.id));
            self.results.extend(results);
        } else {
            self.results = results;
        }
        self.rounds += 1;
        self.checked_at = Some(now);
    }

    /// The table every text surface prints: one block per assertion with its state and proof,
    /// grouped by milestone for a mission.
    pub fn table_lines(&self) -> Vec<String> {
        if let Some(lines) = self.mission_table_lines() {
            return lines;
        }
        let mut out = Vec::new();
        for a in &self.assertions {
            self.assertion_lines(a, &mut out);
        }
        out
    }

    fn assertion_lines(&self, a: &Assertion, out: &mut Vec<String>) {
        let (mark, state) = match self.result(a.id) {
            Some(r) if r.unverified() => ("?", "UNVERIFIED"),
            Some(r) if r.passed => ("✓", "pass"),
            Some(_) => ("✗", "FAIL"),
            None => ("·", "not run"),
        };
        out.push(format!("{mark} {}. {} [{state}]", a.id, a.text));
        out.push(format!("     check: {}", a.command));
        if !a.expect.is_empty() {
            out.push(format!("     expect: {}", a.expect));
        }
        if let Some(r) = self.result(a.id) {
            if (!r.passed || r.unverified()) && !r.reason.is_empty() {
                out.push(format!("     why: {}", r.reason));
            }
            if let Some(last) = r.proof.lines().rev().find(|l| !l.trim().is_empty()) {
                out.push(format!("     proof: {}", clip(last.trim(), 160)));
            }
            if let Some(path) = &r.artifact {
                out.push(format!("     screenshot: {path}"));
            }
        }
    }
}

pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// The last `max` characters of `text`, marked when cut.
pub fn tail(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    let start = chars.len().saturating_sub(max);
    let body: String = chars[start..].iter().collect();
    if start > 0 {
        format!("…{body}")
    } else {
        body
    }
}

pub(crate) fn text_of(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn first_of<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|k| map.get(*k))
}

/// One assertion from a model-written object. The check may be a bare command string or an
/// object `{command, expect, contains}`. `None` when there is no statement or no command: an
/// assertion nobody can run proves nothing.
fn assertion_from(value: &serde_json::Value, id: u32) -> Option<Assertion> {
    let map = value.as_object()?;
    let text = text_of(first_of(
        map,
        &[
            "assertion",
            "text",
            "statement",
            "claim",
            "title",
            "description",
        ],
    ));
    let behaviour = map
        .get("behaviour")
        .and_then(behaviour::Behaviour::from_value);
    let check = first_of(map, &["check", "command", "cmd"]);
    let (command, mut expect, mut contains) = match check {
        Some(serde_json::Value::String(s)) => (s.trim().to_string(), String::new(), None),
        Some(serde_json::Value::Object(c)) => (
            text_of(first_of(c, &["command", "cmd", "run"])),
            text_of(first_of(c, &["expect", "expected", "outcome"])),
            Some(text_of(c.get("contains"))).filter(|s| !s.is_empty()),
        ),
        _ => (String::new(), String::new(), None),
    };
    if expect.is_empty() {
        expect = text_of(first_of(map, &["expect", "expected", "outcome"]));
    }
    if contains.is_none() {
        contains = Some(text_of(map.get("contains"))).filter(|s| !s.is_empty());
    }
    if let Some(b) = behaviour {
        if text.is_empty() {
            return None;
        }
        return Some(Assertion {
            id,
            text: clip(&text, TEXT_CHARS),
            command: clip(&b.summary(), COMMAND_CHARS),
            expect: b.expect.clone(),
            contains: None,
            behaviour: Some(b),
        });
    }
    if text.is_empty() || command.is_empty() {
        return None;
    }
    Some(Assertion {
        id,
        text: clip(&text, TEXT_CHARS),
        command: clip(&command, COMMAND_CHARS),
        expect: clip(&expect, TEXT_CHARS),
        contains: contains.map(|c| clip(&c, TEXT_CHARS)),
        behaviour: None,
    })
}

fn assertion_list(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(items) => Some(items),
        serde_json::Value::Object(map) => map.get("assertions").and_then(|v| v.as_array()),
        _ => None,
    }
}

/// Parse a model's contract reply: the first JSON object with an `assertions` array (or a bare
/// array), tolerant of code fences and prose around it. Assertions without a statement or a
/// command are dropped, the rest renumbered from 1 and capped at [`MAX_ASSERTIONS`]. `None`
/// when fewer than [`MIN_ASSERTIONS`] usable ones remain.
pub fn parse_contract(text: &str) -> Option<Vec<Assertion>> {
    let items = text
        .char_indices()
        .filter(|(_, c)| matches!(c, '{' | '['))
        .find_map(|(i, _)| {
            let mut stream =
                serde_json::Deserializer::from_str(&text[i..]).into_iter::<serde_json::Value>();
            match stream.next() {
                Some(Ok(value)) => assertion_list(&value).cloned(),
                _ => None,
            }
        })?;
    let mut out: Vec<Assertion> = Vec::new();
    for item in &items {
        if let Some(a) = assertion_from(item, out.len() as u32 + 1) {
            out.push(a);
        }
        if out.len() == MAX_ASSERTIONS {
            break;
        }
    }
    (out.len() >= MIN_ASSERTIONS).then_some(out)
}

/// The card shown for approval, as plan-card steps: the statement, and the check with its
/// expected outcome as the detail.
pub fn card_steps(assertions: &[Assertion]) -> Vec<(String, String)> {
    assertions
        .iter()
        .map(|a| {
            let mut detail = format!("check: {}", a.command);
            if !a.expect.is_empty() {
                detail.push_str(&format!(" → {}", a.expect));
            }
            if let Some(c) = &a.contains {
                detail.push_str(&format!(" (output contains \"{c}\")"));
            }
            (a.text.clone(), detail)
        })
        .collect()
}

/// What the worker is told once the contract is approved, so it builds toward the checks.
pub fn worker_guidance(contract: &GoalContract) -> String {
    if let Some(text) = mission::worker_guidance(contract) {
        return text;
    }
    let mut out = String::from(
        "This goal has an approved validation contract. An independent validator runs every \
         check itself after each turn; the goal is done only when all of them pass. Do not \
         claim completion on your own judgement.\n",
    );
    for a in &contract.assertions {
        out.push_str(&format!("{}. {} — check: {}\n", a.id, a.text, a.command));
    }
    out
}

/// One failing assertion as the worker is told: the command and the tail of its output.
fn failure_line(contract: &GoalContract, id: u32) -> Option<String> {
    let a = contract.assertions.iter().find(|a| a.id == id)?;
    let (reason, proof) = match contract.result(id) {
        Some(r) => (r.reason.as_str(), r.proof.as_str()),
        None => ("not run", ""),
    };
    let detail = if proof.trim().is_empty() {
        reason
    } else {
        proof
    };
    Some(format!(
        "assertion {id} failed: {} -> {}\n  ({})\n",
        a.command,
        tail(detail, PROOF_CHARS).replace('\n', "\n    "),
        a.text
    ))
}

/// The next worker prompt after a validator run with failures: one line per failing assertion
/// with the command and the tail of its output.
pub fn failure_prompt(goal: &str, contract: &GoalContract) -> String {
    if let Some(text) = mission::failure_prompt(goal, contract) {
        return text;
    }
    let mut out = format!(
        "Goal: {goal}\nThe independent validator ran the goal contract: {} of {} assertions \
         pass. Fix what fails, then stop; the validator re-runs every check.\n",
        contract.passed_count(),
        contract.assertions.len()
    );
    for id in contract.failing_ids() {
        if let Some(line) = failure_line(contract, id) {
            out.push_str(&line);
        }
    }
    out
}

#[cfg(test)]
#[path = "goal_contract_tests.rs"]
mod tests;
