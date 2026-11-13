//! `forge goal show`: the morning view of a `/goal` run recorded by the daemon — its state and,
//! for a `--contract` goal, the contract table with the validator's proof
//! (docs/features/goal-contract.md).

use anyhow::{bail, Context, Result};

use super::GoalRecord;

/// The text `forge goal show` prints for one stored goal record.
pub(crate) fn render_record(session_id: &str, json: &str) -> Result<String> {
    let record: GoalRecord =
        serde_json::from_str(json).context("reading the stored goal record")?;
    let phase = record
        .phase
        .as_deref()
        .unwrap_or("paused")
        .replace('_', " ");
    let mut out = format!(
        "🎯 goal {phase} — {}\n   session {session_id} · iteration {} · {} tokens · {}s\n",
        record.objective, record.iteration, record.tokens_used, record.elapsed_secs
    );
    if let Some(reason) = &record.reason {
        out.push_str(&format!("   {reason}\n"));
    }
    match &record.contract {
        Some(contract) => {
            out.push_str(&format!(
                "\n📜 contract — {} (validator ran {} time{})\n",
                contract.headline(),
                contract.rounds,
                if contract.rounds == 1 { "" } else { "s" }
            ));
            for line in contract.table_lines() {
                out.push_str(&line);
                out.push('\n');
            }
        }
        None => out.push_str(
            "\nThis goal has no validation contract (start one with /goal --contract).\n",
        ),
    }
    Ok(out)
}

pub(crate) fn goal_show_cmd(session: Option<&str>, json: bool) -> Result<()> {
    let store = crate::open_store()?;
    let (id, stored) = match session {
        Some(prefix) => {
            let mut matches = store.matching_session_ids(prefix)?;
            let id = match matches.len() {
                0 => bail!("no session matches '{prefix}' — see `forge sessions`"),
                1 => matches.remove(0),
                n => bail!("'{prefix}' is ambiguous ({n} sessions) — use more characters"),
            };
            let Some(stored) = store.session_goal_state(&id)? else {
                bail!("session {id} has no goal on record");
            };
            (id, stored)
        }
        None => store
            .latest_goal_state()?
            .context("no goal on record — goals run by the daemon are recorded here")?,
    };
    if json {
        println!("{stored}");
    } else {
        print!("{}", render_record(&id, &stored)?);
        let links = super::goal_relay::relay_links(&store, &id);
        if let Some(chain) = super::goal_relay::chain_line(&links) {
            let ids: Vec<String> = links
                .iter()
                .enumerate()
                .map(|(i, l)| format!("S{} {}", i + 1, l.session_id))
                .collect();
            println!("\nfresh-worker relay: {chain}\n   {}", ids.join(" · "));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::goal_contract::mission::parse_mission;
    use forge_types::goal_contract::{parse_contract, AssertionResult, GoalContract};

    fn record_json(passed: bool) -> String {
        let text = r#"{"assertions":[
          {"assertion":"a exists","check":"test -f a"},
          {"assertion":"b exists","check":"test -f b"},
          {"assertion":"c exists","check":"test -f c"}]}"#;
        let mut contract = GoalContract::new(parse_contract(text).unwrap());
        contract.approved = true;
        contract.record(
            (1..=3)
                .map(|id| AssertionResult {
                    id,
                    passed: passed || id != 2,
                    proof: "test: b: No such file".into(),
                    reason: if passed || id != 2 {
                        String::new()
                    } else {
                        "exit 1".into()
                    },
                    artifact: None,
                })
                .collect(),
            1,
        );
        serde_json::json!({
            "objective": "ship it", "gates": [], "max_tokens": null, "max_minutes": null,
            "iteration": 4, "tokens_used": 900, "elapsed_secs": 61, "reason": null,
            "envelope": null, "contract": contract, "phase": "failed"
        })
        .to_string()
    }

    #[test]
    fn the_morning_view_prints_state_and_the_contract_table() {
        let text = render_record("s1", &record_json(false)).unwrap();
        assert!(text.contains("goal failed — ship it"), "{text}");
        assert!(
            text.contains("contract — 2/3 pass (validator ran 1 time)"),
            "{text}"
        );
        assert!(text.contains("✗ 2. b exists [FAIL]"), "{text}");
        assert!(text.contains("proof: test: b: No such file"), "{text}");
    }

    #[test]
    fn a_goal_without_a_contract_says_so() {
        let mut value: serde_json::Value = serde_json::from_str(&record_json(true)).unwrap();
        value["contract"] = serde_json::Value::Null;
        let text = render_record("s1", &value.to_string()).unwrap();
        assert!(text.contains("no validation contract"), "{text}");
    }

    #[test]
    fn a_mission_prints_its_progress_line_and_milestone_groups() {
        let reply = r#"{"milestones":[
          {"title":"lexer","assertions":[{"assertion":"a","check":"true"},{"assertion":"b","check":"true"}]},
          {"title":"parser","assertions":[{"assertion":"c","check":"true"},{"assertion":"d","check":"true"}]}]}"#;
        let mut contract = parse_mission(reply, 6).unwrap().into_contract().unwrap();
        contract.approved = true;
        let pass = |id| AssertionResult {
            id,
            passed: true,
            proof: String::new(),
            reason: String::new(),
            artifact: None,
        };
        contract.record(vec![pass(1), pass(2)], 1);
        contract.advance_milestone();
        let mut value: serde_json::Value = serde_json::from_str(&record_json(false)).unwrap();
        value["contract"] = serde_json::to_value(&contract).unwrap();
        let text = render_record("s1", &value.to_string()).unwrap();
        assert!(
            text.contains("contract — milestone 2/2: 0/2 pass"),
            "{text}"
        );
        assert!(text.contains("milestone 1/2 - lexer [done 2/2]"), "{text}");
        assert!(
            text.contains("milestone 2/2 - parser [current 0/2 pass]"),
            "{text}"
        );
    }
}
