//! The mission side of the `/goal` loop (docs/features/goal-contract.md, "Missions (milestones)"):
//! the `--mission` flag, and the step from one milestone to the next once the validator has
//! passed the current one and every earlier one still holds.

use forge_types::goal_contract::mission::milestone_prompt;

use super::*;

/// Remove a bare `--mission` word from a goal's text. True when it was there. It rides along in
/// the objective from the slash-command parser to the goal start, so it is read here.
pub(crate) fn take_flag(text: &str) -> (String, bool) {
    let mut found = false;
    let kept: Vec<&str> = text
        .split_whitespace()
        .filter(|word| {
            let is_flag = *word == "--mission";
            found |= is_flag;
            !is_flag
        })
        .collect();
    if found {
        (kept.join(" "), true)
    } else {
        (text.trim().to_string(), false)
    }
}

/// After a validator run: when the current milestone and every earlier one pass and there is a
/// next milestone, move the contract on and return the prompt that milestone starts with. The
/// number of the milestone that passed is left on the track for the relay.
pub(crate) fn advance_if_passed(state: &mut GoalState) -> Option<String> {
    let contract = state.contract.as_mut()?;
    contract.mission_ref()?;
    if !contract.active_all_passed() || contract.all_passed() {
        return None;
    }
    let passed = contract.mission_ref()?.current;
    if !contract.advance_milestone() {
        return None;
    }
    state.validation.milestone_passed = Some(passed);
    state.validation.last_failing.clear();
    state.validation.unchanged = 0;
    milestone_prompt(&state.goal, state.contract.as_ref()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_is_removed_wherever_it_stands_and_only_as_a_whole_word() {
        assert_eq!(
            take_flag("--mission ship the parser"),
            ("ship the parser".to_string(), true)
        );
        assert_eq!(
            take_flag("ship the parser --mission"),
            ("ship the parser".to_string(), true)
        );
        assert_eq!(
            take_flag("explain --missionary work"),
            ("explain --missionary work".to_string(), false)
        );
    }
}
