//! The TUI side of `/contract`: the goal's validation contract as a table, one row per assertion
//! with pass or fail and the last line of proof. Enter on a row prints the check and the whole
//! proof the validator kept (docs/features/goal-contract.md).

use forge_tui::{PickerKind, PickerRow};
use forge_types::goal_contract::GoalContract;

const PROOF_PREVIEW: usize = 90;

fn mark(contract: &GoalContract, id: u32) -> &'static str {
    match contract.result(id) {
        Some(r) if r.passed => "pass",
        Some(_) => "FAIL",
        None => "not run",
    }
}

fn last_line(text: &str) -> &str {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
}

pub(crate) fn rows(contract: &GoalContract) -> Vec<PickerRow> {
    contract
        .assertions
        .iter()
        .map(|a| {
            let state = mark(contract, a.id);
            let detail = match contract.result(a.id) {
                Some(r) if !r.passed && !r.reason.is_empty() => r.reason.clone(),
                Some(r) => last_line(&r.proof).to_string(),
                None => a.command.clone(),
            };
            let preview: String = detail.chars().take(PROOF_PREVIEW).collect();
            let milestone = contract
                .mission_ref()
                .and_then(|m| m.milestone_of(a.id).map(|n| format!("M{n}/{} ", m.total())))
                .unwrap_or_default();
            PickerRow {
                id: format!("assertion:{}", a.id),
                title: format!("{milestone}{}. {} [{state}]", a.id, a.text),
                subtitle: preview,
            }
        })
        .collect()
}

pub(crate) fn open(app: &mut forge_tui::App) {
    let Some(contract) = app.goal.as_ref().and_then(|g| g.contract.clone()) else {
        app.note("no goal contract — start one with /goal --contract <objective>");
        return;
    };
    let title = format!(
        "goal contract - {} (validator ran {} time{})",
        contract.headline(),
        contract.rounds,
        if contract.rounds == 1 { "" } else { "s" }
    );
    app.picker
        .open_with(PickerKind::Contract, &title, rows(&contract));
}

/// The check and the full proof of one assertion, as a transcript note.
pub(crate) fn detail(contract: &GoalContract, id: u32) -> Option<String> {
    let a = contract.assertions.iter().find(|a| a.id == id)?;
    let mut out = format!(
        "assertion {id} [{}]: {}\n  check: {}",
        mark(contract, id),
        a.text,
        a.command
    );
    if !a.expect.is_empty() {
        out.push_str(&format!("\n  expect: {}", a.expect));
    }
    if let Some(r) = contract.result(id) {
        if !r.reason.is_empty() {
            out.push_str(&format!("\n  why: {}", r.reason));
        }
        if r.proof.trim().is_empty() {
            out.push_str("\n  proof: (no output)");
        } else {
            out.push_str(&format!(
                "\n  proof:\n    {}",
                r.proof.trim().replace('\n', "\n    ")
            ));
        }
    }
    Some(out)
}

pub(crate) fn accept(app: &mut forge_tui::App, row: &PickerRow) {
    let Some(id) = row
        .id
        .strip_prefix("assertion:")
        .and_then(|n| n.parse::<u32>().ok())
    else {
        return;
    };
    let text = app
        .goal
        .as_ref()
        .and_then(|g| g.contract.as_ref())
        .and_then(|c| detail(c, id));
    if let Some(text) = text {
        app.note(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::goal_contract::{Assertion, AssertionResult};

    fn contract() -> GoalContract {
        let mut c = GoalContract::new(
            (1..=3)
                .map(|id| Assertion {
                    id,
                    text: format!("thing {id} exists"),
                    command: format!("test -f t{id}"),
                    expect: "exit 0".into(),
                    contains: None,
                })
                .collect(),
        );
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
                    proof: "line one\ntest: t2: No such file".into(),
                    reason: "exit 1".into(),
                },
            ],
            1,
        );
        c
    }

    #[test]
    fn the_table_has_a_row_per_assertion_with_its_state() {
        let rows = rows(&contract());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].title, "1. thing 1 exists [pass]");
        assert_eq!(rows[1].title, "2. thing 2 exists [FAIL]");
        assert_eq!(rows[1].subtitle, "exit 1");
        assert_eq!(rows[2].title, "3. thing 3 exists [not run]");
        assert_eq!(rows[2].subtitle, "test -f t3");
        assert_eq!(rows[1].id, "assertion:2");
    }

    #[test]
    fn enter_shows_the_whole_proof_and_the_overlay_opens_only_with_a_contract() {
        let mut app = forge_tui::App::default();
        open(&mut app);
        assert!(!app.picker.open, "no contract, no overlay");

        app.goal_begin("ship it", &[], None, None);
        app.goal_set_contract(contract());
        open(&mut app);
        assert!(app.picker.open);
        let text = detail(&contract(), 2).unwrap();
        assert!(text.contains("check: test -f t2"), "{text}");
        assert!(text.contains("why: exit 1"), "{text}");
        assert!(
            text.contains("    line one\n    test: t2: No such file"),
            "{text}"
        );
        assert!(detail(&contract(), 9).is_none());
    }

    #[test]
    fn mission_rows_say_which_milestone_each_assertion_belongs_to() {
        let reply = r#"{"milestones":[
          {"title":"x","assertions":[{"assertion":"a","check":"true"},{"assertion":"b","check":"true"}]},
          {"title":"y","assertions":[{"assertion":"c","check":"true"},{"assertion":"d","check":"true"}]}]}"#;
        let contract = forge_types::goal_contract::mission::parse_mission(reply, 6)
            .unwrap()
            .into_contract()
            .unwrap();
        let rows = rows(&contract);
        assert_eq!(rows[0].title, "M1/2 1. a [not run]");
        assert_eq!(rows[3].title, "M2/2 4. d [not run]");
        let mut app = forge_tui::App::default();
        app.goal_begin("g", &[], None, None);
        app.goal_set_contract(contract);
        open(&mut app);
        assert!(app.picker.open);
    }
}
