use super::*;

fn plan(items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "summary": "split the work", "items": items })
}

fn st(index: usize, status: &str, deps: &[usize]) -> ItemState {
    ItemState {
        index,
        status: status.to_string(),
        depends_on: deps.to_vec(),
    }
}

#[test]
fn a_valid_plan_parses_with_trimmed_fields_and_sorted_unique_dependencies() {
    let p = parse_plan(
        &plan(serde_json::json!([
            { "title": "  API  ", "prompt": " add the endpoint " },
            { "title": "Tests", "prompt": "test it", "depends_on": [1, 1] }
        ])),
        8,
    )
    .unwrap();
    assert_eq!(p.summary, "split the work");
    assert_eq!(p.items[0].title, "API");
    assert_eq!(p.items[0].prompt, "add the endpoint");
    assert_eq!(p.items[1].depends_on, vec![1]);
}

#[test]
fn every_rejection_names_what_to_fix() {
    let cases = [
        (
            serde_json::json!({ "items": [{ "title": "a", "prompt": "b" }] }),
            "summary",
        ),
        (plan(serde_json::json!([])), "at least one"),
        (plan(serde_json::json!([{ "prompt": "b" }])), "no title"),
        (plan(serde_json::json!([{ "title": "a" }])), "no prompt"),
        (
            plan(
                serde_json::json!([{ "title": "a", "prompt": "b" }, { "title": "A", "prompt": "c" }]),
            ),
            "distinct title",
        ),
        (
            plan(serde_json::json!([{ "title": "a", "prompt": "b", "depends_on": [2] }])),
            "does not exist",
        ),
        (
            plan(serde_json::json!([{ "title": "a", "prompt": "b", "depends_on": [1] }])),
            "itself",
        ),
        (
            plan(serde_json::json!([
                { "title": "a", "prompt": "b", "depends_on": [2] },
                { "title": "c", "prompt": "d", "depends_on": [1] }
            ])),
            "cycle",
        ),
        (
            plan(serde_json::json!([{ "title": "a", "prompt": "b", "depends_on": "1" }])),
            "array",
        ),
    ];
    for (args, needle) in cases {
        let err = parse_plan(&args, 8).unwrap_err();
        assert!(err.contains(needle), "{err:?} should mention {needle:?}");
    }
}

#[test]
fn the_item_cap_is_the_hosts_and_never_above_the_hard_ceiling() {
    let three = plan(serde_json::json!([
        { "title": "a", "prompt": "x" },
        { "title": "b", "prompt": "x" },
        { "title": "c", "prompt": "x" }
    ]));
    assert!(parse_plan(&three, 2).unwrap_err().contains("at most 2"));
    assert!(parse_plan(&three, 3).is_ok());
    let many: Vec<_> = (0..MAX_ITEMS_HARD + 1)
        .map(|i| serde_json::json!({ "title": format!("t{i}"), "prompt": "x" }))
        .collect();
    let err = parse_plan(&plan(serde_json::Value::Array(many)), 100).unwrap_err();
    assert!(err.contains(&format!("at most {MAX_ITEMS_HARD}")), "{err}");
}

#[test]
fn over_long_titles_and_prompts_are_refused() {
    let long_title = "x".repeat(MAX_TITLE_CHARS + 1);
    assert!(parse_plan(
        &plan(serde_json::json!([{ "title": long_title, "prompt": "p" }])),
        8
    )
    .unwrap_err()
    .contains("longer than"));
    let long_prompt = "x".repeat(MAX_PROMPT_BYTES + 1);
    assert!(parse_plan(
        &plan(serde_json::json!([{ "title": "t", "prompt": long_prompt }])),
        8
    )
    .unwrap_err()
    .contains("byte limit"));
}

#[test]
fn approving_everything_queues_everything() {
    let a = resolve_selection(&[vec![], vec![1], vec![]], None).unwrap();
    assert_eq!(a.queued, vec![1, 2, 3]);
    assert!(a.skipped.is_empty());
    assert!(a.dropped_for_deps.is_empty());
}

#[test]
fn deselecting_a_dependency_drops_its_dependents_transitively() {
    // 2 needs 1, 3 needs 2. Select 2 and 3 only.
    let a = resolve_selection(&[vec![], vec![1], vec![2]], Some(&[2, 3])).unwrap();
    assert!(a.queued.is_empty());
    assert_eq!(a.skipped, vec![1]);
    assert_eq!(a.dropped_for_deps, vec![2, 3]);
}

#[test]
fn a_selection_must_name_real_items_and_not_be_empty() {
    assert!(resolve_selection(&[vec![]], Some(&[])).is_err());
    assert!(resolve_selection(&[vec![]], Some(&[2])).is_err());
    assert!(resolve_selection(&[vec![]], Some(&[0])).is_err());
}

#[test]
fn scheduling_respects_the_running_cap_in_order() {
    let items = [
        st(1, item_status::QUEUED, &[]),
        st(2, item_status::QUEUED, &[]),
        st(3, item_status::QUEUED, &[]),
    ];
    assert_eq!(schedule(&items, 2).start, vec![1, 2]);
    let items = [
        st(1, item_status::RUNNING, &[]),
        st(2, item_status::QUEUED, &[]),
        st(3, item_status::QUEUED, &[]),
    ];
    assert_eq!(schedule(&items, 2).start, vec![2]);
}

#[test]
fn a_dependent_waits_until_its_dependency_succeeds_or_is_merged() {
    let waiting = [
        st(1, item_status::RUNNING, &[]),
        st(2, item_status::QUEUED, &[1]),
    ];
    assert_eq!(schedule(&waiting, 4), Schedule::default());
    for done in [item_status::SUCCEEDED, item_status::MERGED] {
        let ready = [st(1, done, &[]), st(2, item_status::QUEUED, &[1])];
        assert_eq!(schedule(&ready, 4).start, vec![2], "{done}");
    }
}

#[test]
fn a_failed_dependency_cancels_the_whole_chain_behind_it() {
    let items = [
        st(1, item_status::FAILED, &[]),
        st(2, item_status::QUEUED, &[1]),
        st(3, item_status::QUEUED, &[2]),
        st(4, item_status::QUEUED, &[]),
    ];
    let s = schedule(&items, 4);
    assert_eq!(s.cancel, vec![2, 3]);
    assert_eq!(s.start, vec![4]);
}

#[test]
fn a_zero_cap_still_lets_one_session_run() {
    assert_eq!(
        schedule(&[st(1, item_status::QUEUED, &[])], 0).start,
        vec![1]
    );
}

#[test]
fn all_finished_only_when_every_item_is_terminal() {
    assert!(all_finished([item_status::SUCCEEDED, item_status::SKIPPED]));
    assert!(!all_finished([item_status::SUCCEEDED, item_status::QUEUED]));
}

#[test]
fn the_coordinator_prompt_carries_the_request_and_names_no_mock_intent() {
    let p = coordinator_prompt("  make it faster  ", "/repo", 5, true);
    assert!(p.contains("<request>\nmake it faster\n</request>"));
    assert!(p.contains("/repo"));
    assert!(p.contains("at most 5"));
    assert!(p.contains("own git worktree"));
    assert!(p.contains("dispatch_sessions"));
    // Item phrasing lines up with the worker's turn contract: an instruction verb demands a diff,
    // the read-only sentence forbids one.
    assert!(p.contains("starting with a verb such as Add, Implement, Fix"));
    assert!(p.contains("\"Do not edit files.\""));
    // The offline mock provider keys on these phrases; a coordinator prompt containing one would
    // make every mock coordinator take the wrong branch regardless of the user's request.
    let lower = p.to_lowercase();
    for trigger in [
        "present_plan",
        "step-by-step plan",
        "ordered plan",
        "update_tasks",
        "task list",
        "track tasks",
        "code block",
        "create a file",
        "mock:",
    ] {
        assert!(
            !lower.contains(trigger),
            "coordinator prompt contains {trigger:?}"
        );
    }
    assert!(coordinator_prompt("x", "/r", 5, false).contains("same working directory"));
}

#[test]
fn the_worker_prompt_puts_the_item_first_and_names_its_neighbours() {
    let siblings = [(1, "API"), (3, "Docs")];
    let p = worker_prompt(&WorkerContext {
        summary: "ship the feature",
        coordinator_title: "Dispatch: ship it",
        coordinator_id: "abcdef1234567890",
        index: 2,
        total: 3,
        title: "Tests",
        prompt: "write the tests",
        siblings: &siblings,
        worktree: true,
    });
    assert!(p.starts_with("write the tests\n\n---\n"));
    assert!(p.contains("session 2 of 3"));
    assert!(p.contains("1. API; 3. Docs"));
    assert!(p.contains("target \"abcdef12\""));
    assert!(p.contains("own git worktree"));
}

#[test]
fn coordinator_messages_say_what_happened_and_what_to_do_next() {
    let deps: &[usize] = &[1];
    let m = approved_message(
        &[(1, "API", "sessionaaaa")],
        &[(2, "Tests", deps)],
        &[(3, "Docs")],
    );
    assert!(m.starts_with("[dispatch]"));
    assert!(m.contains("1. API (sessiona)"));
    assert!(m.contains("2. Tests (after 1)"));
    assert!(m.contains("Not started:\n- 3. Docs"));

    assert!(revise_message(" fewer items ").contains("\n\nfewer items\n\n"));
    assert!(cancelled_message(0).ends_with("with one line."));
    assert!(cancelled_message(2).contains("The 2 sessions already running keep running."));

    let long = "r".repeat(REPORT_MAX_CHARS * 2);
    let cancelled = [(3, "Docs")];
    let f = item_finished_message(&FinishedReport {
        index: 2,
        total: 3,
        title: "Tests",
        session_id: "session-bbbb",
        outcome: "failed",
        stop_reason: Some("max_steps"),
        last_reply: &long,
        still_running: 1,
        still_waiting: 0,
        cancelled: &cancelled,
    });
    assert!(f.contains("Session 2/3 \"Tests\" (session-) stopped without finishing (max steps)"));
    assert!(f.contains("did not succeed: 3. Docs."));
    assert!(f.len() < long.len(), "the report is truncated");

    let done = all_finished_message(&[
        (1, "API", item_status::SUCCEEDED),
        (2, "Tests", item_status::FAILED),
    ]);
    assert!(done.contains("- 1. API: succeeded"));
    assert!(done.contains("- 2. Tests: stopped without finishing"));
}

#[test]
fn the_tool_spec_clamps_its_advertised_cap() {
    let spec = dispatch_sessions_spec(99);
    assert_eq!(spec.name, DISPATCH_SESSIONS_TOOL);
    assert_eq!(
        spec.schema["properties"]["items"]["maxItems"],
        MAX_ITEMS_HARD
    );
    assert_eq!(
        dispatch_sessions_spec(0).schema["properties"]["items"]["maxItems"],
        1
    );
}
