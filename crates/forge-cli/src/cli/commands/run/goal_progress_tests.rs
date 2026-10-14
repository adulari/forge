use super::*;

fn verdict(achieved: bool, remaining: &[&str], next: &str) -> GoalVerdict {
    GoalVerdict {
        achieved,
        remaining: remaining.iter().map(|s| s.to_string()).collect(),
        next_step: next.to_string(),
        confidence: 0.9,
        progressed: true,
        reason: if achieved {
            "all parts land".into()
        } else {
            "parts missing".into()
        },
        model: "test::model".into(),
    }
}

fn state(gates: &[&str]) -> GoalState {
    GoalState::new(
        1,
        "add three subcommands".into(),
        gates.iter().map(|g| g.to_string()).collect(),
        None,
        None,
    )
}

async fn decide(
    gs: &mut GoalState,
    said_complete: bool,
    done: usize,
    total: usize,
    v: Option<GoalVerdict>,
) -> GoalDecision {
    let dir = tempfile::tempdir().unwrap();
    next_goal_decision(gs, said_complete, done, total, 100, v, dir.path()).await
}

fn retry_prompt(decision: GoalDecision) -> Option<String> {
    match decision {
        GoalDecision::Retry { prompt } => prompt,
        GoalDecision::Stop(reason) => panic!("expected a retry, got stop: {reason}"),
        GoalDecision::GateFailed { prompt } => panic!("expected a retry, got gate: {prompt}"),
    }
}

#[tokio::test]
async fn an_achieved_verdict_stops_once_the_gates_pass() {
    let marker = tempfile::tempdir().unwrap();
    let ran = marker.path().join("gate-ran");
    let gate = format!("touch '{}'", ran.display());
    let mut gs = state(&[gate.as_str()]);
    let d = decide(&mut gs, false, 1, 3, Some(verdict(true, &[], ""))).await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if is_goal_complete_reason(r)));
    assert!(ran.exists(), "the gate ran before the run stopped");
}

#[tokio::test]
async fn an_achieved_verdict_with_a_failing_gate_redrives_the_gate() {
    let mut gs = state(&["false"]);
    let d = decide(&mut gs, false, 3, 3, Some(verdict(true, &[], ""))).await;
    assert!(
        matches!(d, GoalDecision::GateFailed { ref prompt } if prompt.contains("`false`")),
        "a judged completion still has to pass its gates"
    );
    assert!(gs.eval.last_gate_failure.is_some());

    let d = decide(
        &mut gs,
        false,
        3,
        3,
        Some(verdict(false, &["fix gate"], "fix gate")),
    )
    .await;
    let prompt = retry_prompt(d).unwrap();
    assert!(
        prompt.contains("Last quality gate failure: `false` failed"),
        "{prompt}"
    );
}

#[tokio::test]
async fn an_unfinished_verdict_builds_the_next_prompt_from_what_is_missing() {
    let mut gs = state(&[]);
    let v = verdict(
        false,
        &["add the `stats` subcommand", "test the `rm` subcommand"],
        "add the `stats` subcommand",
    );
    let prompt = retry_prompt(decide(&mut gs, false, 1, 3, Some(v)).await).unwrap();
    assert!(prompt.starts_with("Goal: add three subcommands"));
    assert!(prompt.contains("- add the `stats` subcommand"));
    assert!(prompt.contains("- test the `rm` subcommand"));
    assert!(prompt.contains("Next step: add the `stats` subcommand"));
    assert!(prompt.contains("Goal evaluator: parts missing"));
    assert!(!prompt.contains(GOAL_CONTINUE_PROMPT));
    assert!(
        prompt.lines().count() <= 10,
        "the re-drive prompt stays short"
    );
}

#[tokio::test]
async fn no_verdict_falls_back_to_the_fixed_prompt_and_stall_counter() {
    assert!(forge_core::goal_evaluator::parse_goal_verdict("looks done to me").is_none());
    let mut gs = state(&[]);
    for turn in 1..GOAL_NO_PROGRESS_MAX {
        let d = decide(&mut gs, false, 0, 3, None).await;
        assert_eq!(
            retry_prompt(d),
            None,
            "turn {turn} uses GOAL_CONTINUE_PROMPT"
        );
    }
    let d = decide(&mut gs, false, 0, 3, None).await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if r.contains("stalled")));
}

#[tokio::test]
async fn no_verdict_keeps_the_task_plan_completion() {
    let mut gs = state(&[]);
    let d = decide(&mut gs, false, 3, 3, None).await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if is_goal_complete_reason(r)));
}

#[tokio::test]
async fn a_claimed_completion_the_evaluator_rejects_continues_then_stops_bounded() {
    let mut gs = state(&[]);
    for round in 1..=GOAL_DISAGREE_MAX {
        // Vary the list so this exercises the disagreement bound, not wedge detection.
        let item = format!("write the README section {round}");
        let v = verdict(false, &[item.as_str()], &item);
        let prompt = retry_prompt(decide(&mut gs, true, 3, 3, Some(v)).await).unwrap();
        assert!(
            prompt
                .contains("You reported the goal complete, but the goal evaluator says it is not"),
            "{prompt}"
        );
        assert!(prompt.contains(&item));
    }
    let v = verdict(false, &["write the README for real"], "write it");
    let d = decide(&mut gs, true, 3, 3, Some(v)).await;
    assert!(
        matches!(d, GoalDecision::Stop(ref r) if r.contains("evaluator disagrees") && !is_goal_complete_reason(r)),
        "the disagreement loop is bounded and never reported as success"
    );
}

#[tokio::test]
async fn a_low_confidence_achieved_verdict_needs_the_agent_to_agree() {
    let mut gs = state(&[]);
    let mut v = verdict(true, &[], "");
    v.confidence = 0.2;
    let d = decide(&mut gs, false, 1, 3, Some(v.clone())).await;
    assert!(retry_prompt(d)
        .unwrap()
        .contains("confirm every part of the goal is met"));
    let d = decide(&mut gs, true, 3, 3, Some(v)).await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if is_goal_complete_reason(r)));
}

#[tokio::test]
async fn an_unchanging_remaining_list_stops_the_goal_as_wedged() {
    let mut gs = state(&[]);
    let first = verdict(false, &["Add the stats command", "add tests"], "add stats");
    retry_prompt(decide(&mut gs, false, 0, 3, Some(first)).await);
    for _ in 1..GOAL_EVAL_WEDGED_TURNS {
        // Same items, reordered and recased: still no change.
        let same = verdict(false, &["add tests", "add the stats command."], "add stats");
        retry_prompt(decide(&mut gs, false, 0, 3, Some(same)).await);
    }
    let same = verdict(false, &["add tests", "add the stats command"], "add stats");
    let d = decide(&mut gs, false, 0, 3, Some(same)).await;
    assert!(
        matches!(d, GoalDecision::Stop(ref r) if r.starts_with("🎯 goal wedged") && r.contains("parts missing")),
        "wedged after {GOAL_EVAL_WEDGED_TURNS} unchanged verdicts"
    );
}

#[tokio::test]
async fn a_shrinking_list_resets_the_wedge_streak() {
    let mut gs = state(&[]);
    let v = verdict(false, &["a", "b", "c"], "a");
    for _ in 0..GOAL_EVAL_WEDGED_TURNS {
        retry_prompt(decide(&mut gs, false, 0, 3, Some(v.clone())).await);
    }
    assert_eq!(gs.eval.unchanged, GOAL_EVAL_WEDGED_TURNS - 1);
    retry_prompt(decide(&mut gs, false, 1, 3, Some(verdict(false, &["b", "c"], "b"))).await);
    assert_eq!(gs.eval.unchanged, 0);
}

#[tokio::test]
async fn an_explicit_no_progress_verdict_counts_toward_wedged() {
    let mut gs = state(&[]);
    for i in 0..GOAL_EVAL_WEDGED_TURNS {
        let item = format!("item {i}");
        let mut v = verdict(false, &[item.as_str()], &item);
        v.progressed = false;
        let d = decide(&mut gs, false, 0, 3, Some(v)).await;
        if i + 1 == GOAL_EVAL_WEDGED_TURNS {
            assert!(matches!(d, GoalDecision::Stop(ref r) if r.contains("wedged")));
        }
    }
}

#[tokio::test]
async fn the_iteration_ceiling_still_applies_with_verdicts() {
    let mut gs = state(&[]);
    gs.iter = GOAL_MAX_ITERS;
    let d = decide(&mut gs, false, 0, 3, Some(verdict(false, &["x"], "x"))).await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if r.contains("iteration ceiling")));
}

#[tokio::test]
async fn the_token_budget_still_applies_with_verdicts() {
    let mut gs = GoalState::new(1, "g".into(), Vec::new(), Some(50), None);
    let d = decide(&mut gs, false, 0, 3, Some(verdict(false, &["x"], "x"))).await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if r.contains("token budget")));
}

#[test]
fn the_evaluator_request_carries_the_previous_list_and_gate_failure() {
    let mut gs = state(&[]);
    gs.iter = 4;
    gs.eval.last = Some(verdict(false, &["add docs"], "add docs"));
    gs.eval.last_gate_failure = Some("`pytest` failed: 1 failed".into());
    let request = gs.eval_request();
    assert_eq!(request.goal, "add three subcommands");
    assert_eq!(request.iteration, 4);
    assert_eq!(request.previous_remaining, vec!["add docs"]);
    assert!(request.last_gate_failure.unwrap().contains("pytest"));
    assert!(request.started_unix > 0);
}
