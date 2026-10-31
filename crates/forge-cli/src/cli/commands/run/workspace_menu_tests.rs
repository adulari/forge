use super::*;

fn run(arg: &str) -> serde_json::Value {
    match parse_worktree_args(arg) {
        Ok(WtCommand::Run(v)) => v,
        other => panic!("{arg}: {other:?}"),
    }
}

#[test]
fn bare_command_opens_the_overlay() {
    assert_eq!(parse_worktree_args(""), Ok(WtCommand::Open));
    assert_eq!(parse_worktree_args("  "), Ok(WtCommand::Open));
}

#[test]
fn new_takes_an_optional_name_and_carry() {
    assert_eq!(run("new"), json!({"action": "create"}));
    assert_eq!(
        run("new calm-otter"),
        json!({"action": "create", "name": "calm-otter"})
    );
    assert_eq!(
        run("new --carry"),
        json!({"action": "create", "dirty": "carry"})
    );
    assert_eq!(
        run("new fix-x --carry"),
        json!({"action": "create", "name": "fix-x", "dirty": "carry"})
    );
}

#[test]
fn new_rejects_bad_names_and_unknown_flags() {
    assert!(parse_worktree_args("new bad name").is_err());
    assert!(parse_worktree_args("new ../x").is_err());
    assert!(parse_worktree_args("new x --force").is_err());
}

#[test]
fn verbs_map_to_the_tool_requests() {
    assert_eq!(run("list"), json!({"action": "list"}));
    assert_eq!(run("exit"), json!({"action": "exit"}));
    assert_eq!(
        run("enter feature"),
        json!({"action": "enter", "path": "feature"})
    );
    assert_eq!(
        run("remove feature"),
        json!({"action": "remove", "path": "feature"})
    );
    assert_eq!(parse_worktree_args("pull"), Ok(WtCommand::Pull));
}

#[test]
fn malformed_input_gets_the_usage_line() {
    for bad in ["enter", "enter a b", "remove", "list extra", "frobnicate"] {
        let err = parse_worktree_args(bad).unwrap_err();
        assert!(err.starts_with("usage: /worktree"), "{bad}: {err}");
    }
}

#[test]
fn every_parsed_request_is_one_the_tool_accepts() {
    for arg in [
        "new",
        "new a --carry",
        "list",
        "enter x",
        "exit",
        "remove x",
    ] {
        assert!(Request::parse(&run(arg)).is_ok(), "{arg}");
    }
}
