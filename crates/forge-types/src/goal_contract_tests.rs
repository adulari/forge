use super::*;

fn three() -> String {
    r#"{"assertions": [
        {"assertion": "todo.py exists", "check": "test -f todo.py", "expect": "exit 0"},
        {"assertion": "has a done subcommand", "check": {"command": "grep -n 'def done' todo.py", "contains": "def done"}},
        {"text": "tests dir present", "command": "ls tests", "expected": "lists files"}
    ]}"#
    .to_string()
}

#[test]
fn parses_object_check_string_check_and_alias_keys() {
    let a = parse_contract(&three()).unwrap();
    assert_eq!(a.len(), 3);
    assert_eq!(a[0].id, 1);
    assert_eq!(a[0].command, "test -f todo.py");
    assert_eq!(a[1].contains.as_deref(), Some("def done"));
    assert_eq!(a[2].text, "tests dir present");
    assert_eq!(a[2].expect, "lists files");
}

#[test]
fn tolerates_fences_prose_and_a_bare_array() {
    let fenced = format!("Here is the contract:\n```json\n{}\n```\nDone.", three());
    assert_eq!(parse_contract(&fenced).unwrap().len(), 3);
    let bare = r#"[{"assertion":"a","check":"true"},{"assertion":"b","check":"true"},{"assertion":"c","check":"true"}]"#;
    assert_eq!(parse_contract(bare).unwrap().len(), 3);
}

#[test]
fn drops_assertions_without_a_command_and_renumbers() {
    let text = r#"{"assertions":[
        {"assertion":"a","check":"true"},
        {"assertion":"the UI feels fast"},
        {"assertion":"b","check":"true"},
        {"assertion":"c","check":"true"}]}"#;
    let a = parse_contract(text).unwrap();
    assert_eq!(a.iter().map(|x| x.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(a[1].text, "b");
}

#[test]
fn fewer_than_three_usable_assertions_is_no_contract() {
    let text =
        r#"{"assertions":[{"assertion":"a","check":"true"},{"assertion":"b","check":"true"}]}"#;
    assert!(parse_contract(text).is_none());
    assert!(parse_contract("no json at all").is_none());
}

#[test]
fn caps_at_twelve() {
    let items: Vec<String> = (0..20)
        .map(|i| format!(r#"{{"assertion":"a{i}","check":"true"}}"#))
        .collect();
    let text = format!(r#"{{"assertions":[{}]}}"#, items.join(","));
    assert_eq!(parse_contract(&text).unwrap().len(), MAX_ASSERTIONS);
}

fn contract() -> GoalContract {
    GoalContract::new(parse_contract(&three()).unwrap())
}

fn result(id: u32, passed: bool, proof: &str) -> AssertionResult {
    AssertionResult {
        id,
        passed,
        proof: proof.to_string(),
        reason: if passed {
            String::new()
        } else {
            "exit 1".to_string()
        },
        artifact: None,
    }
}

#[test]
fn all_passed_needs_a_passing_result_for_every_assertion() {
    let mut c = contract();
    assert!(!c.all_passed());
    c.record(vec![result(1, true, ""), result(2, true, "")], 10);
    assert!(!c.all_passed(), "assertion 3 has no result");
    c.record(
        vec![
            result(1, true, ""),
            result(2, true, ""),
            result(3, false, "x"),
        ],
        11,
    );
    assert_eq!(c.failing_ids(), vec![3]);
    assert_eq!(c.headline(), "2/3 pass");
    c.record(
        vec![
            result(1, true, ""),
            result(2, true, ""),
            result(3, true, ""),
        ],
        12,
    );
    assert!(c.all_passed());
    assert_eq!(c.rounds, 3);
}

#[test]
fn failure_prompt_names_the_assertion_command_and_output_tail() {
    let mut c = contract();
    c.record(
        vec![
            result(1, true, ""),
            result(2, false, "grep: todo.py: No such file"),
            result(3, true, ""),
        ],
        1,
    );
    let prompt = failure_prompt("build the todo cli", &c);
    assert!(
        prompt.contains(
            "assertion 2 failed: grep -n 'def done' todo.py -> grep: todo.py: No such file"
        ),
        "{prompt}"
    );
    assert!(prompt.contains("2 of 3"), "{prompt}");
    assert!(!prompt.contains("assertion 1 failed"));
}

#[test]
fn table_marks_pass_fail_and_not_run() {
    let mut c = contract();
    assert!(c.table_lines()[0].contains("[not run]"));
    c.record(vec![result(1, true, "ok"), result(2, false, "boom")], 1);
    let table = c.table_lines().join("\n");
    assert!(table.contains("✓ 1."));
    assert!(table.contains("✗ 2. has a done subcommand [FAIL]"));
    assert!(table.contains("proof: boom"));
}

#[test]
fn contract_round_trips_through_json_with_defaults() {
    let c = contract();
    let json = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<GoalContract>(&json).unwrap(), c);
    let old: GoalContract = serde_json::from_str(r#"{"assertions":[]}"#).unwrap();
    assert!(!old.approved && old.results.is_empty());
}

#[test]
fn a_behaviour_assertion_parses_without_a_read_only_check() {
    let reply = r#"{"assertions":[
      {"assertion":"a exists","check":"test -f a"},
      {"assertion":"b exists","check":"test -f b"},
      {"assertion":"/health says ok v2","behaviour":{"run":"python3 s.py $FORGE_QA_PORT",
        "probe":"GET http://127.0.0.1:$FORGE_QA_PORT/health","expect":"ok v2","cleanup":"rm -f x"}},
      {"assertion":"no probe","behaviour":{"run":"x"}}]}"#;
    let parsed = parse_contract(reply).unwrap();
    assert_eq!(parsed.len(), 3, "a behaviour without a probe is dropped");
    let b = parsed[2].behaviour.as_ref().unwrap();
    assert_eq!(b.expect, "ok v2");
    assert_eq!(b.cleanup, "rm -f x");
    assert!(parsed[2].command.starts_with("behaviour: run `python3"));
    assert!(parsed[0].behaviour.is_none());
}

#[test]
fn a_result_with_a_screenshot_prints_its_path() {
    let mut c = GoalContract::new(vec![Assertion {
        id: 1,
        text: "page shows the button".into(),
        command: "behaviour: run `x`, probe `screenshot u`".into(),
        expect: String::new(),
        contains: None,
        behaviour: None,
    }]);
    let mut r = result(1, true, "ok");
    r.artifact = Some("/ev/qa-1.png".into());
    c.record(vec![r], 1);
    assert!(c
        .table_lines()
        .iter()
        .any(|l| l.contains("screenshot: /ev/qa-1.png")));
}
