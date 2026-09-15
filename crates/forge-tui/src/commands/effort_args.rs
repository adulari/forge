//! Arg parsing for `/effort` — the ceiling pin and the `exact` rung override. Its own submodule so
//! the `parse_command` arm in `commands.rs` stays a one-liner: that file sits at its CI file-size
//! ratchet ceiling (`scripts/ci/architecture_size.py`), so new logic has to live in new files
//! rather than grow it.

use super::CommandAction;

/// Turn `/effort <arg>` into its action.
///
/// `exact` is the ONLY word that changes what the command means; every other argument stays the
/// ceiling pin it has always been, so `/effort high` is untouched. The level itself stays raw text:
/// validating it belongs with the surface that knows the routed model, since which rungs exist
/// depends on that model's provider.
pub(crate) fn parse_effort_arg(arg: &str) -> CommandAction {
    let trimmed = arg.trim();
    let Some(rest) = trimmed.strip_prefix("exact") else {
        return CommandAction::SetEffort((!arg.is_empty()).then(|| arg.to_string()));
    };
    // `exactly` is a value, not the subcommand: only a bare `exact` or `exact <...>` counts.
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return CommandAction::SetEffort((!arg.is_empty()).then(|| arg.to_string()));
    }
    // `off`/`none` clear it, so the override can be lifted without restarting the session.
    let rest = rest.trim();
    let clearing =
        rest.is_empty() || rest.eq_ignore_ascii_case("off") || rest.eq_ignore_ascii_case("none");
    CommandAction::SetExactEffort((!clearing).then(|| rest.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_switches_the_meaning_and_nothing_else_does() {
        assert_eq!(
            parse_effort_arg("exact high"),
            CommandAction::SetExactEffort(Some("high".into()))
        );
        assert_eq!(
            parse_effort_arg("exact max"),
            CommandAction::SetExactEffort(Some("max".into()))
        );
        // Three ways to clear, so the override never becomes sticky by accident.
        assert_eq!(
            parse_effort_arg("exact"),
            CommandAction::SetExactEffort(None)
        );
        assert_eq!(
            parse_effort_arg("exact off"),
            CommandAction::SetExactEffort(None)
        );
        assert_eq!(
            parse_effort_arg("exact NONE"),
            CommandAction::SetExactEffort(None)
        );
    }

    #[test]
    fn every_other_argument_is_still_the_ceiling_pin() {
        assert_eq!(
            parse_effort_arg("high"),
            CommandAction::SetEffort(Some("high".into()))
        );
        assert_eq!(parse_effort_arg(""), CommandAction::SetEffort(None));
        // A word merely STARTING with "exact" is a value, not the subcommand.
        assert_eq!(
            parse_effort_arg("exactly"),
            CommandAction::SetEffort(Some("exactly".into()))
        );
    }
}
