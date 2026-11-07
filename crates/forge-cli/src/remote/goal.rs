//! The `/goal` run on the wire: state, budget and the request a command left pending, so a
//! client can show a chip and offer pause, resume and stop without parsing transcript notes.

use forge_tui::goal_status::GoalStatus;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapGoal {
    pub objective: String,
    /// `active`, `paused`, `budget_limit`, `complete`, `failed` or `cancelled`.
    pub state: &'static str,
    pub iteration: u64,
    pub tokens_used: u64,
    pub max_tokens: Option<u64>,
    pub max_minutes: Option<u64>,
    /// Run time as of `synced_at`; an `active` goal keeps running past it.
    pub elapsed_secs: u64,
    pub synced_at: u64,
    pub gates: Vec<String>,
    /// Why a goal that did not complete stopped.
    pub reason: Option<String>,
    /// `pause` or `clear`, applied when the turn in flight ends.
    pub pending: Option<&'static str>,
    /// The spend envelope (`/goal --budget`); absent when the goal has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<SnapEnvelope>,
    /// The validation contract (`/goal --contract`) with the validator's latest results; absent
    /// when the goal has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<SnapContract>,
    /// The sessions the goal has run in, oldest first (fresh-worker relay); absent for a goal that
    /// never changed worker.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relay: Vec<SnapRelay>,
}

/// One session of the relay chain.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapRelay {
    pub session_id: String,
    /// `handed_off` for a session that gave the goal on, else what the goal is doing there.
    pub state: &'static str,
    /// `wedged`, `stalled` or `worn`: why it handed the goal on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A goal contract as the clients draw it: a table, one row per assertion.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapContract {
    /// The ready-to-show line, e.g. `2/3 pass`.
    pub headline: String,
    pub approved: bool,
    /// How many times the validator has run.
    pub rounds: u32,
    pub passed: usize,
    pub total: usize,
    pub checked_at: Option<i64>,
    pub assertions: Vec<SnapAssertion>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapAssertion {
    pub id: u32,
    pub text: String,
    pub command: String,
    pub expect: String,
    /// `pass`, `fail`, or `pending` before the validator has run.
    pub state: &'static str,
    pub reason: String,
    /// The tail of the check's output.
    pub proof: String,
}

impl From<&forge_types::goal_contract::GoalContract> for SnapContract {
    fn from(contract: &forge_types::goal_contract::GoalContract) -> Self {
        Self {
            headline: contract.headline(),
            approved: contract.approved,
            rounds: contract.rounds,
            passed: contract.passed_count(),
            total: contract.assertions.len(),
            checked_at: contract.checked_at,
            assertions: contract
                .assertions
                .iter()
                .map(|a| {
                    let result = contract.result(a.id);
                    SnapAssertion {
                        id: a.id,
                        text: a.text.clone(),
                        command: a.command.clone(),
                        expect: a.expect.clone(),
                        state: match result {
                            Some(r) if r.passed => "pass",
                            Some(_) => "fail",
                            None => "pending",
                        },
                        reason: result.map(|r| r.reason.clone()).unwrap_or_default(),
                        proof: result.map(|r| r.proof.clone()).unwrap_or_default(),
                    }
                })
                .collect(),
        }
    }
}

/// A goal's spend envelope: how much of its slice is used, ready to draw as a fill bar.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SnapEnvelope {
    /// `percent` of a subscription window, or `usd`.
    pub unit: &'static str,
    pub used: f64,
    pub slice: f64,
    /// `claude weekly`, `codex 5h` or `spend`.
    pub label: String,
    /// The ready-to-show line, e.g. `envelope 12/20% claude weekly`.
    pub meter: String,
    pub spent: bool,
    /// The last check found no fresh reading to measure against.
    pub unverified: bool,
    /// When the measured window resets, Unix seconds.
    pub resets_at: Option<i64>,
    /// When a goal paused on this envelope continues by itself (the window reset), Unix seconds.
    /// Absent when it will not: the user paused it, it is not spent, or the reset time is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resumes_at: Option<i64>,
}

impl From<&GoalStatus> for SnapGoal {
    fn from(goal: &GoalStatus) -> Self {
        Self {
            objective: goal.objective.clone(),
            state: goal.phase.wire(),
            iteration: goal.iteration as u64,
            tokens_used: goal.tokens_used,
            max_tokens: goal.max_tokens,
            max_minutes: goal.max_minutes,
            elapsed_secs: goal.elapsed_secs,
            synced_at: goal.synced_at_ms,
            gates: goal.gates.clone(),
            reason: goal.reason.clone(),
            pending: goal.request.map(|request| request.wire()),
            envelope: goal.envelope.as_ref().map(|e| SnapEnvelope {
                unit: if e.is_usd() { "usd" } else { "percent" },
                used: e.used,
                slice: e.spec.slice,
                label: e.spec.label(),
                meter: e.meter(),
                spent: e.spent,
                unverified: e.unverified,
                resets_at: e.resets_at,
                resumes_at: e.auto_resume_at(),
            }),
            contract: goal.contract.as_ref().map(SnapContract::from),
            relay: goal
                .relay
                .iter()
                .map(|link| SnapRelay {
                    session_id: link.session_id.clone(),
                    state: if link.handed_off.is_some() {
                        "handed_off"
                    } else {
                        goal.phase.wire()
                    },
                    reason: link.handed_off.clone(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::goal_contract::{parse_contract, AssertionResult, GoalContract};

    #[test]
    fn the_snapshot_carries_the_relay_chain_and_omits_it_without_one() {
        use forge_tui::goal_status::GoalRelayLink;
        let mut app = forge_tui::App::default();
        app.goal_begin("ship it", &[], None, None);
        let bare = serde_json::to_value(SnapGoal::from(app.goal.as_ref().unwrap())).unwrap();
        assert!(bare.get("relay").is_none());
        app.goal_set_relay(vec![
            GoalRelayLink {
                session_id: "aaaa".into(),
                handed_off: Some("wedged".into()),
            },
            GoalRelayLink {
                session_id: "bbbb".into(),
                handed_off: None,
            },
        ]);
        let json = serde_json::to_value(SnapGoal::from(app.goal.as_ref().unwrap())).unwrap();
        assert_eq!(json["relay"][0]["state"], "handed_off");
        assert_eq!(json["relay"][0]["reason"], "wedged");
        assert_eq!(json["relay"][1]["state"], "active");
        assert!(json["relay"][1].get("reason").is_none());
        assert_eq!(
            app.goal.as_ref().unwrap().relay_line().as_deref(),
            Some("S1 handed off (wedged) -> S2 running")
        );
        assert!(app
            .goal
            .as_ref()
            .unwrap()
            .summary()
            .contains("S1 handed off (wedged) -> S2 running"));
    }

    #[test]
    fn the_snapshot_carries_the_contract_table_and_omits_it_without_one() {
        let mut app = forge_tui::App::default();
        app.goal_begin("ship it", &[], None, None);
        let bare = serde_json::to_value(SnapGoal::from(app.goal.as_ref().unwrap())).unwrap();
        assert!(bare.get("contract").is_none());

        let text = r#"{"assertions":[
          {"assertion":"a exists","check":"test -f a","expect":"exit 0"},
          {"assertion":"b exists","check":"test -f b"},
          {"assertion":"c exists","check":"test -f c"}]}"#;
        let mut contract = GoalContract::new(parse_contract(text).unwrap());
        contract.approved = true;
        app.goal_set_contract(contract);
        let before = serde_json::to_value(SnapGoal::from(app.goal.as_ref().unwrap())).unwrap();
        assert_eq!(before["contract"]["assertions"][0]["state"], "pending");
        app.goal_contract_results(vec![
            AssertionResult {
                id: 1,
                passed: true,
                proof: String::new(),
                reason: String::new(),
            },
            AssertionResult {
                id: 2,
                passed: false,
                proof: "no such file".into(),
                reason: "exit 1".into(),
            },
        ]);
        let v = serde_json::to_value(SnapGoal::from(app.goal.as_ref().unwrap())).unwrap();
        let c = &v["contract"];
        assert_eq!(c["headline"], "1/3 pass");
        assert_eq!(
            (
                c["passed"].as_u64(),
                c["total"].as_u64(),
                c["rounds"].as_u64()
            ),
            (Some(1), Some(3), Some(1))
        );
        assert_eq!(c["assertions"][0]["state"], "pass");
        assert_eq!(c["assertions"][0]["expect"], "exit 0");
        assert_eq!(c["assertions"][1]["state"], "fail");
        assert_eq!(c["assertions"][1]["reason"], "exit 1");
        assert_eq!(c["assertions"][1]["proof"], "no such file");
        assert_eq!(c["assertions"][2]["state"], "pending");
    }
}
