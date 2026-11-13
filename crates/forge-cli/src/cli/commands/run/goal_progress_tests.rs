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

mod envelope {
    use super::*;
    use forge_types::goal_envelope::{EnvelopeSpec, EnvelopeState};

    fn facts(windows: Vec<forge_store::SubscriptionWindow>, now: i64) -> GoalTurnFacts {
        GoalTurnFacts {
            done: 0,
            total: 3,
            said_complete: false,
            budget_refused: false,
            run_budget_hit: false,
            root: std::env::temp_dir(),
            sources: EnvelopeSources {
                windows,
                session_usd: 0.0,
                now,
            },
            relay: None,
        }
    }

    fn weekly(pct: f64, updated_at: i64) -> forge_store::SubscriptionWindow {
        forge_store::SubscriptionWindow {
            provider: "claude-cli".into(),
            window_kind: "weekly".into(),
            status: "ok".into(),
            resets_at: Some(updated_at + 86_400),
            fraction: Some(pct / 100.0),
            updated_at,
        }
    }

    fn enveloped(start: f64) -> GoalState {
        let mut gs = state(&[]);
        gs.envelope = Some(EnvelopeState::new(
            EnvelopeSpec::parse("claude:20%w", None).unwrap(),
            start,
            None,
        ));
        gs
    }

    #[tokio::test]
    async fn readings_rising_past_the_slice_pause_the_goal() {
        let mut gs = enveloped(40.0);
        let step = settle_goal(&mut gs, facts(vec![weekly(50.0, 1000)], 1000)).await;
        assert!(matches!(step, GoalStep::Continue(_) | GoalStep::Turn(_)));
        assert_eq!(
            gs.envelope.as_ref().unwrap().meter(),
            "envelope 10/20% claude weekly"
        );

        let step = settle_goal(&mut gs, facts(vec![weekly(60.0, 2000)], 2000)).await;
        let GoalStep::Pause { reason } = step else {
            panic!("expected a pause at the cap");
        };
        assert!(
            reason.starts_with("Envelope spent (20% of Claude weekly)."),
            "{reason}"
        );
        assert!(gs.envelope.as_ref().unwrap().spent);
    }

    #[tokio::test]
    async fn a_stale_reading_pauses_instead_of_spending_blind() {
        let mut gs = enveloped(40.0);
        let step = settle_goal(&mut gs, facts(vec![weekly(41.0, 0)], 100_000)).await;
        let GoalStep::Pause { reason } = step else {
            panic!("expected a pause without a fresh reading");
        };
        assert!(reason.contains("can't be verified"), "{reason}");
    }

    #[tokio::test]
    async fn a_goal_without_an_envelope_is_untouched() {
        let mut gs = state(&[]);
        let step = settle_goal(&mut gs, facts(Vec::new(), 1000)).await;
        assert!(matches!(step, GoalStep::Continue(_) | GoalStep::Turn(_)));
    }

    #[tokio::test]
    async fn a_usd_envelope_counts_session_cost_since_the_goal_started() {
        let mut gs = state(&[]);
        gs.envelope = Some(EnvelopeState::new(
            EnvelopeSpec::parse("$2", None).unwrap(),
            10.0,
            None,
        ));
        let mut f = facts(Vec::new(), 1000);
        f.sources.session_usd = 11.0;
        assert!(matches!(
            settle_goal(&mut gs, f).await,
            GoalStep::Continue(_)
        ));
        assert_eq!(gs.budget.usd_remaining, Some(1.0));
        let mut f = facts(Vec::new(), 1000);
        f.sources.session_usd = 12.5;
        assert!(matches!(
            settle_goal(&mut gs, f).await,
            GoalStep::Pause { .. }
        ));
    }
}

#[test]
fn a_turn_that_ended_provider_down_pauses_the_goal_without_counting_it() {
    let mut app = forge_tui::App::default();
    app.goal_begin("ship it", &[], None, None);
    app.apply(forge_tui::PresenterEvent::Done {
        final_text: String::new(),
        stop_reason: forge_types::StopReason::ProviderDown,
    });
    assert!(pause_if_provider_down(&mut app));
    let goal = app
        .goal
        .as_ref()
        .expect("the goal is kept for /goal resume");
    assert_eq!(goal.phase, forge_tui::goal_status::GoalPhase::Paused);
    assert!(goal
        .reason
        .as_deref()
        .is_some_and(|r| r.contains("provider stayed down")));
}

#[test]
fn every_other_stop_reason_leaves_the_goal_running() {
    for reason in [
        forge_types::StopReason::FinalAnswer,
        forge_types::StopReason::NoOutput,
        forge_types::StopReason::Stalled,
    ] {
        let mut app = forge_tui::App::default();
        app.goal_begin("ship it", &[], None, None);
        app.apply(forge_tui::PresenterEvent::Done {
            final_text: String::new(),
            stop_reason: reason,
        });
        assert!(!pause_if_provider_down(&mut app), "{reason:?}");
    }
}

fn scratch_repo_with_ci() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    crate::ci_run::opt_in_ci_gate(dir.path());
    std::fs::write(
        wf.join("ci.yml"),
        "on: pull_request\nenv:\n  STRICT: \"1\"\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - name: Strict check\n        run: test \"$STRICT\" = 1 && sh ./check.sh\n      - name: Needs a secret\n        env:\n          T: ${{ secrets.T }}\n        run: echo $T\n  win:\n    runs-on: windows-latest\n    steps:\n      - run: echo hi\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("check.sh"),
        "echo lint-failed >&2\nexit 1\n",
    )
    .unwrap();
    dir
}

#[tokio::test]
async fn a_goal_is_not_done_until_the_repos_ci_plan_passes() {
    let repo = scratch_repo_with_ci();
    let mut gs = state(&[]);
    let done = verdict(true, &[], "");
    let first =
        next_goal_decision(&mut gs, false, 1, 1, 100, Some(done.clone()), repo.path()).await;
    let GoalDecision::GateFailed { prompt } = first else {
        panic!("a failing CI plan must not let the goal finish");
    };
    assert!(prompt.contains("forge ci run"), "{prompt}");
    assert!(
        prompt.contains("Local CI step failed: Strict check"),
        "{prompt}"
    );
    assert!(
        prompt.contains("STRICT=1") && prompt.contains("lint-failed"),
        "{prompt}"
    );

    std::fs::write(repo.path().join("check.sh"), "exit 0\n").unwrap();
    let second = next_goal_decision(&mut gs, false, 1, 1, 100, Some(done), repo.path()).await;
    assert!(matches!(second, GoalDecision::Stop(ref r) if is_goal_complete_reason(r)));
}

#[tokio::test]
async fn gate_off_in_ci_toml_lets_the_goal_finish_without_running_the_plan() {
    let repo = scratch_repo_with_ci();
    std::fs::create_dir_all(repo.path().join(".forge")).unwrap();
    std::fs::write(repo.path().join(".forge/ci.toml"), "[ci]\ngate = \"off\"\n").unwrap();
    let mut gs = state(&[]);
    let d = next_goal_decision(
        &mut gs,
        false,
        1,
        1,
        100,
        Some(verdict(true, &[], "")),
        repo.path(),
    )
    .await;
    assert!(matches!(d, GoalDecision::Stop(ref r) if is_goal_complete_reason(r)));
    assert!(gs.gates.is_empty());
}

#[tokio::test]
async fn a_contract_goal_also_waits_for_the_ci_plan() {
    let repo = scratch_repo_with_ci();
    let mut gs = state(&[]);
    let d = finish_with_gates(&mut gs, GOAL_COMPLETE_REASON, repo.path()).await;
    assert!(matches!(d, GoalDecision::GateFailed { .. }));
}

mod mission {
    use super::*;
    use forge_types::goal_contract::mission::parse_mission;

    fn mission_state() -> GoalState {
        let items = |t: &str| {
            format!(
                r#"{{"title":"{t}","assertions":[{{"assertion":"{t} a","check":"true"}},{{"assertion":"{t} b","check":"true"}}]}}"#
            )
        };
        let reply = format!(
            r#"{{"milestones":[{},{},{}]}}"#,
            items("lexer"),
            items("parser"),
            items("cli")
        );
        let mut contract = parse_mission(&reply, 6).unwrap().into_contract().unwrap();
        contract.approved = true;
        let mut gs = state(&[]);
        gs.contract = Some(contract);
        gs
    }

    fn validated(gs: &mut GoalState, passing: &[u32]) {
        let contract = gs.contract.as_mut().unwrap();
        let results = contract
            .active_assertions()
            .map(|a| forge_types::goal_contract::AssertionResult {
                id: a.id,
                passed: passing.contains(&a.id),
                proof: if passing.contains(&a.id) {
                    String::new()
                } else {
                    "missing".into()
                },
                reason: if passing.contains(&a.id) {
                    String::new()
                } else {
                    "exit 1".into()
                },
                artifact: None,
            })
            .collect();
        contract.record(results, 1);
        gs.validation.fresh = true;
    }

    fn current(gs: &GoalState) -> u32 {
        gs.contract.as_ref().unwrap().mission_ref().unwrap().current
    }

    #[tokio::test]
    async fn a_passing_milestone_moves_the_mission_on_with_the_next_milestones_prompt() {
        let mut gs = mission_state();
        validated(&mut gs, &[1, 2]);
        let prompt = retry_prompt(decide(&mut gs, false, 0, 0, None).await).unwrap();
        assert!(
            prompt.contains("Milestone 1 passed. Now milestone 2/3: parser"),
            "{prompt}"
        );
        assert_eq!(current(&gs), 2);
        assert_eq!(gs.validation.milestone_passed, Some(1));
        assert_eq!(
            gs.contract.as_ref().unwrap().headline(),
            "milestone 2/3: 0/2 pass"
        );
    }

    #[tokio::test]
    async fn a_failing_milestone_goes_back_to_the_worker_and_stays_put() {
        let mut gs = mission_state();
        validated(&mut gs, &[1]);
        let prompt = retry_prompt(decide(&mut gs, true, 0, 0, None).await).unwrap();
        assert!(prompt.contains("assertion 2 failed"), "{prompt}");
        assert!(!prompt.contains("regression"), "{prompt}");
        assert_eq!(current(&gs), 1);
        assert_eq!(gs.validation.milestone_passed, None);
    }

    #[tokio::test]
    async fn a_broken_earlier_assertion_becomes_the_prompt_and_blocks_the_boundary() {
        let mut gs = mission_state();
        gs.contract.as_mut().unwrap().advance_milestone();
        // Milestone 2 is done, but assertion 1 of milestone 1 no longer holds.
        validated(&mut gs, &[2, 3, 4]);
        let prompt = retry_prompt(decide(&mut gs, false, 0, 0, None).await).unwrap();
        assert!(
            prompt.contains("REGRESSION (milestone 1) assertion 1 failed"),
            "{prompt}"
        );
        assert!(prompt.contains("Fix the regression first"), "{prompt}");
        assert_eq!(
            current(&gs),
            2,
            "the mission does not advance over a regression"
        );
        assert_eq!(gs.validation.milestone_passed, None);
    }

    #[tokio::test]
    async fn the_mission_is_achieved_only_when_the_last_milestone_and_all_earlier_ones_pass() {
        let mut gs = mission_state();
        gs.contract.as_mut().unwrap().advance_milestone();
        gs.contract.as_mut().unwrap().advance_milestone();
        validated(&mut gs, &[1, 2, 3, 4, 5]);
        let decision = decide(&mut gs, true, 0, 0, None).await;
        assert!(
            matches!(decision, GoalDecision::Retry { .. }),
            "milestone 3 itself fails"
        );
        let mut gs = mission_state();
        gs.contract.as_mut().unwrap().advance_milestone();
        gs.contract.as_mut().unwrap().advance_milestone();
        validated(&mut gs, &[2, 3, 4, 5, 6]);
        let decision = decide(&mut gs, true, 0, 0, None).await;
        assert!(
            matches!(decision, GoalDecision::Retry { .. }),
            "a regression in milestone 1 blocks the finish"
        );
        let mut gs = mission_state();
        gs.contract.as_mut().unwrap().advance_milestone();
        gs.contract.as_mut().unwrap().advance_milestone();
        validated(&mut gs, &[1, 2, 3, 4, 5, 6]);
        match decide(&mut gs, false, 0, 0, None).await {
            GoalDecision::Stop(reason) => assert!(is_goal_complete_reason(&reason), "{reason}"),
            _ => panic!("expected the mission to complete"),
        }
    }

    #[tokio::test]
    async fn the_wedge_rule_counts_only_the_active_failing_set() {
        let mut gs = mission_state();
        for round in 0..GOAL_EVAL_WEDGED_TURNS {
            validated(&mut gs, &[1]);
            let decision = decide(&mut gs, false, 0, 0, None).await;
            if round + 1 < GOAL_EVAL_WEDGED_TURNS {
                retry_prompt(decision);
            } else {
                match decision {
                    GoalDecision::Stop(reason) => {
                        assert!(reason.contains("assertions 2 still fail"), "{reason}")
                    }
                    _ => panic!("expected a wedge"),
                }
            }
        }
    }
}
