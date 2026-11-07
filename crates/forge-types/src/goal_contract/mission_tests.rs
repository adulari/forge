use super::*;
use crate::goal_contract::{failure_prompt, worker_guidance, AssertionResult};

fn milestone_json(title: &str, n: usize) -> String {
    let items: Vec<String> = (0..n)
        .map(|i| format!(r#"{{"assertion":"{title} fact {i}","check":"test -f {title}{i}"}}"#))
        .collect();
    format!(
        r#"{{"title":"{title}","assertions":[{}]}}"#,
        items.join(",")
    )
}

fn reply(milestones: &[(&str, usize)]) -> String {
    let items: Vec<String> = milestones
        .iter()
        .map(|(t, n)| milestone_json(t, *n))
        .collect();
    format!(r#"{{"milestones":[{}]}}"#, items.join(","))
}

fn mission_contract() -> GoalContract {
    parse_mission(&reply(&[("lexer", 2), ("parser", 3), ("cli", 2)]), 6)
        .unwrap()
        .into_contract()
        .unwrap()
}

fn result(id: u32, passed: bool) -> AssertionResult {
    AssertionResult {
        id,
        passed,
        proof: if passed {
            String::new()
        } else {
            "no such file".into()
        },
        reason: if passed {
            String::new()
        } else {
            "exit 1".into()
        },
    }
}

fn pass_all(c: &mut GoalContract) {
    let ids: Vec<u32> = c.active_assertions().map(|a| a.id).collect();
    c.record(ids.into_iter().map(|id| result(id, true)).collect(), 1);
}

#[test]
fn a_mission_reply_parses_through_fences_prose_and_key_aliases() {
    let text = format!(
        "Here is the plan:\n```json\n{}\n```\nGood luck.",
        r#"{"stages":[
          {"name":"first","checks":[{"text":"a","cmd":"true"},{"statement":"b","command":"true"}]},
          {"title":"second","criteria":[{"claim":"c","check":{"command":"true","expect":"exit 0"}},{"assertion":"d","check":"true","contains":"x"}]}]}"#
    );
    let draft = parse_mission(&text, 6).unwrap();
    assert_eq!(draft.milestones.len(), 2);
    assert_eq!(draft.milestones[0].title, "first");
    assert_eq!(
        draft.milestones[1].assertions[1].contains.as_deref(),
        Some("x")
    );
}

#[test]
fn a_bare_array_of_milestones_parses() {
    let text = format!("[{},{}]", milestone_json("a", 2), milestone_json("b", 2));
    assert_eq!(parse_mission(&text, 6).unwrap().milestones.len(), 2);
}

#[test]
fn milestones_with_too_few_usable_assertions_are_dropped_and_two_must_remain() {
    let one_bad = r#"{"milestones":[
      {"title":"thin","assertions":[{"assertion":"a","check":"true"},{"assertion":"no command"}]},
      {"title":"ok","assertions":[{"assertion":"a","check":"true"},{"assertion":"b","check":"true"}]}]}"#;
    assert!(
        parse_mission(one_bad, 6).is_none(),
        "only one milestone is left"
    );
    let three = reply(&[("a", 2), ("b", 1), ("c", 2)]);
    let draft = parse_mission(&three, 6).unwrap();
    let titles: Vec<&str> = draft.milestones.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(titles, ["a", "c"]);
    assert!(parse_mission("no json at all", 6).is_none());
    assert!(parse_mission(r#"{"assertions":[]}"#, 6).is_none());
}

#[test]
fn the_milestone_and_assertion_counts_are_capped() {
    let many = reply(&[("a", 9), ("b", 2), ("c", 2), ("d", 2), ("e", 2)]);
    let draft = parse_mission(&many, 3).unwrap();
    assert_eq!(draft.milestones.len(), 3, "the configured cap");
    assert_eq!(
        draft.milestones[0].assertions.len(),
        MAX_MILESTONE_ASSERTIONS
    );
    let capped_high = parse_mission(&many, 99).unwrap();
    assert_eq!(capped_high.milestones.len(), 5);
    let capped_low = parse_mission(&many, 0).unwrap();
    assert_eq!(
        capped_low.milestones.len(),
        MIN_MILESTONES,
        "never below two"
    );
}

#[test]
fn the_draft_flattens_into_one_numbered_contract_with_the_first_milestone_current() {
    let c = mission_contract();
    assert_eq!(c.assertions.len(), 7);
    let ids: Vec<u32> = c.assertions.iter().map(|a| a.id).collect();
    assert_eq!(ids, (1..=7).collect::<Vec<u32>>());
    let m = c.mission_ref().unwrap();
    assert_eq!(m.current, 1);
    assert_eq!(m.milestones[1].assertion_ids, vec![3, 4, 5]);
    assert_eq!(m.milestone_of(6), Some(3));
    assert_eq!(c.headline(), "milestone 1/3: 2 assertions");
}

#[test]
fn read_only_retention_drops_thin_milestones() {
    let mut draft = parse_mission(&reply(&[("a", 2), ("b", 3)]), 6).unwrap();
    draft.retain(|a| !a.command.contains("b0") && !a.command.contains("b1"));
    assert!(
        !draft.is_usable(),
        "b is left with one assertion and is dropped"
    );
    assert!(draft.into_contract().is_none());
}

#[test]
fn only_the_current_and_earlier_milestones_are_active() {
    let mut c = mission_contract();
    assert_eq!(c.active_assertions().count(), 2);
    assert_eq!(c.validation_scope().assertions.len(), 2);
    assert!(c.validation_scope().mission.is_none());
    assert_eq!(c.failing_ids(), vec![1, 2], "nothing has run yet");
    assert!(c.advance_milestone());
    assert_eq!(c.active_assertions().count(), 5);
    assert!(c.advance_milestone());
    assert!(!c.advance_milestone(), "no milestone after the last");
    assert_eq!(c.active_assertions().count(), 7);
}

#[test]
fn the_current_milestone_passing_is_not_the_mission_achieved() {
    let mut c = mission_contract();
    pass_all(&mut c);
    assert!(c.active_all_passed());
    assert!(!c.all_passed(), "later milestones have no results");
    c.advance_milestone();
    c.advance_milestone();
    pass_all(&mut c);
    assert!(c.all_passed());
}

#[test]
fn results_of_inactive_assertions_survive_a_scoped_run() {
    let mut c = mission_contract();
    pass_all(&mut c);
    c.advance_milestone();
    c.record(vec![result(1, true), result(2, false), result(3, true)], 2);
    assert_eq!(c.results.len(), 3);
    assert_eq!(c.failing_ids(), vec![2, 4, 5]);
    assert_eq!(c.regressed_ids(), vec![2]);
    assert_eq!(c.rounds, 2);
}

#[test]
fn the_headline_names_the_milestone_and_counts_only_its_assertions() {
    let mut c = mission_contract();
    pass_all(&mut c);
    c.advance_milestone();
    c.record(
        vec![
            result(1, true),
            result(2, true),
            result(3, true),
            result(4, false),
        ],
        2,
    );
    assert_eq!(c.headline(), "milestone 2/3: 1/3 pass");
}

#[test]
fn the_table_groups_assertions_under_their_milestone() {
    let mut c = mission_contract();
    pass_all(&mut c);
    c.advance_milestone();
    let lines = c.table_lines();
    let heads: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("milestone "))
        .collect();
    assert_eq!(heads.len(), 3);
    assert!(
        heads[0].contains("lexer") && heads[0].contains("done 2/2"),
        "{}",
        heads[0]
    );
    assert!(
        heads[1].contains("parser") && heads[1].contains("current 0/3 pass"),
        "{}",
        heads[1]
    );
    assert!(heads[2].contains("upcoming"), "{}", heads[2]);
    let first_head = lines
        .iter()
        .position(|l| l.starts_with("milestone 1/3"))
        .unwrap();
    assert!(lines[first_head + 1].contains("1. lexer fact 0"));
}

#[test]
fn a_regression_is_called_out_and_comes_first_in_the_failure_prompt() {
    let mut c = mission_contract();
    pass_all(&mut c);
    c.advance_milestone();
    c.record(
        vec![
            result(1, false),
            result(2, true),
            result(3, false),
            result(4, true),
            result(5, true),
        ],
        2,
    );
    let prompt = failure_prompt("ship it", &c);
    assert!(prompt.contains("Mission milestone 2/3: parser"), "{prompt}");
    assert!(prompt.contains("a regression"), "{prompt}");
    let regression = prompt
        .find("REGRESSION (milestone 1) assertion 1 failed")
        .unwrap();
    let current = prompt.find("assertion 3 failed").unwrap();
    assert!(regression < current, "{prompt}");
    assert!(prompt.contains("3 of 5 assertions pass"), "{prompt}");
}

#[test]
fn a_failure_without_regression_does_not_mention_one() {
    let mut c = mission_contract();
    c.record(vec![result(1, true), result(2, false)], 1);
    let prompt = failure_prompt("ship it", &c);
    assert!(!prompt.contains("regression"), "{prompt}");
    assert!(prompt.contains("assertion 2 failed"));
}

#[test]
fn the_worker_is_told_its_milestone_and_the_earlier_checks_to_keep() {
    let mut c = mission_contract();
    c.advance_milestone();
    let text = worker_guidance(&c);
    assert!(text.contains("milestone 2/3: parser"), "{text}");
    assert!(text.contains("3. parser fact 0"), "{text}");
    assert!(text.contains("must keep passing"), "{text}");
    assert!(text.contains("1. lexer fact 0"), "{text}");
    assert!(
        !text.contains("cli fact"),
        "later milestones are not the worker's yet"
    );
}

#[test]
fn the_next_milestone_prompt_and_the_brief_lines_say_where_the_mission_stands() {
    let mut c = mission_contract();
    c.advance_milestone();
    let prompt = milestone_prompt("ship it", &c).unwrap();
    assert!(
        prompt.contains("Milestone 1 passed. Now milestone 2/3: parser"),
        "{prompt}"
    );
    let brief = brief_lines(&c).unwrap();
    assert!(brief.contains("milestone 2/3 - parser"), "{brief}");
    assert!(brief.contains("3. cli"), "{brief}");
    assert!(milestone_prompt("g", &GoalContract::new(Vec::new())).is_none());
}

#[test]
fn a_mission_survives_json_and_old_records_load_without_one() {
    let mut c = mission_contract();
    c.approved = true;
    pass_all(&mut c);
    c.advance_milestone();
    let back: GoalContract = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back, c);
    assert_eq!(back.mission_ref().unwrap().current, 2);
    let old: GoalContract =
        serde_json::from_str(r#"{"assertions":[],"approved":true,"results":[]}"#).unwrap();
    assert!(old.mission.is_none());
    let flat = serde_json::to_value(GoalContract::new(Vec::new())).unwrap();
    assert!(
        flat.get("mission").is_none(),
        "a flat contract serializes as before"
    );
}

#[test]
fn a_flat_contract_behaves_exactly_as_before() {
    let flat = GoalContract::new(
        crate::goal_contract::parse_contract(&format!(
            r#"{{"assertions":[{}]}}"#,
            (0..3)
                .map(|i| format!(r#"{{"assertion":"a{i}","check":"true"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        ))
        .unwrap(),
    );
    assert_eq!(flat.active_assertions().count(), 3);
    assert_eq!(flat.headline(), "3 assertions");
    assert_eq!(flat.regressed_ids(), Vec::<u32>::new());
    assert!(!flat.advance_milestone_clone());
}

impl GoalContract {
    fn advance_milestone_clone(&self) -> bool {
        self.clone().advance_milestone()
    }
}
