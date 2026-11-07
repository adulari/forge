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
