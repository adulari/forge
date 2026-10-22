//! What running a resolved command or skill needs beyond its prompt, shared by the TUI dispatcher
//! (`catalog_dispatch.rs`) and `forge run "/name …"` (`one_shot.rs`): the per-turn grant its
//! frontmatter carries (`allowed-tools`, `disallowed-tools`, `effort`, `context: fork`) and the
//! `` !`cmd` `` injections in its text.

use std::path::Path;

use forge_core::skill_shell::{self, ShellPolicy};
use forge_core::{ForkTurn, TurnGrant};
use forge_skills::{Invocation, Scope, TemplateVars};

/// The grant one invocation carries. `allowed-tools` from a project-scope definition count only in
/// a trusted project; `line` is what a forked run records as the user's message.
pub(crate) fn grant_for(
    label: &str,
    invocation: &Invocation,
    scope: Scope,
    model: Option<&str>,
    line: &str,
    workspace: &Path,
    vars: &TemplateVars,
) -> TurnGrant {
    let allow_grants =
        scope != Scope::Project || forge_config::project_trust::is_trusted(workspace);
    let mut grant = TurnGrant::from_invocation(label, invocation, allow_grants, vars);
    grant.fork = invocation.fork.as_ref().map(|fork| ForkTurn {
        agent: fork
            .agent
            .clone()
            .unwrap_or_else(|| "general-purpose".to_string()),
        model: model.map(str::to_string),
        label: line.trim().to_string(),
    });
    grant
}

/// Whether any of the texts has an injected command, so callers can skip building a policy.
pub(crate) fn needs_shell(prompt: &str, guidance: &[String]) -> bool {
    skill_shell::has_injections(prompt) || guidance.iter().any(|g| skill_shell::has_injections(g))
}

/// `prompt` and `guidance` with their injected commands run and replaced. `Err` aborts the
/// invocation with the message to show.
pub(crate) fn run_injections(
    prompt: &str,
    guidance: &[String],
    policy: &ShellPolicy,
) -> Result<(String, Vec<String>), String> {
    let prompt = skill_shell::render(prompt, policy)?;
    let guidance = guidance
        .iter()
        .map(|g| skill_shell::render(g, policy))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((prompt, guidance))
}

/// The policy for a one-shot run, built before its session exists: the loaded config's temper and
/// rules (with `--allowedTools`-style overrides applied), plus the invocation's own grant.
pub(crate) fn one_shot_policy(grant: &TurnGrant, workspace: &Path) -> ShellPolicy {
    let mut config = forge_config::load_in(workspace).unwrap_or_default();
    super::claude_flags::apply(&mut config);
    let mut rules = config.permission_rules();
    rules.extend(grant.rules.iter().cloned());
    ShellPolicy {
        mode: config.permission_mode,
        rules,
        workspace: workspace.to_path_buf(),
        trusted: forge_config::project_trust::is_trusted(workspace),
    }
}

/// A grant computed by `forge run "/name …"` before its session is built, applied to that session
/// by `build_session` (one-shot runs build exactly one session per process).
static ONE_SHOT_GRANT: std::sync::Mutex<Option<TurnGrant>> = std::sync::Mutex::new(None);

pub(crate) fn stash_one_shot_grant(grant: TurnGrant) {
    if let Ok(mut slot) = ONE_SHOT_GRANT.lock() {
        *slot = (!grant.is_empty()).then_some(grant);
    }
}

/// Whether `forge run "/name …"` stashed a grant for the session it is about to build.
pub(crate) fn grant_pending() -> bool {
    ONE_SHOT_GRANT.lock().is_ok_and(|slot| slot.is_some())
}

/// Hand a stashed one-shot grant to the session that will run the turn.
pub(crate) fn apply_one_shot_grant(session: &mut forge_core::Session) {
    let grant = ONE_SHOT_GRANT.lock().ok().and_then(|mut slot| slot.take());
    if let Some(grant) = grant {
        session.grant_next_turn(grant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fork_frontmatter_becomes_a_fork_turn() {
        let invocation = Invocation {
            fork: Some(forge_skills::ForkSpec { agent: None }),
            ..Invocation::default()
        };
        let grant = grant_for(
            "skill s",
            &invocation,
            Scope::User,
            Some("haiku"),
            " /s 12 ",
            Path::new("/"),
            &TemplateVars::default(),
        );
        assert_eq!(
            grant.fork,
            Some(ForkTurn {
                agent: "general-purpose".into(),
                model: Some("haiku".into()),
                label: "/s 12".into(),
            })
        );
    }

    #[test]
    fn needs_shell_only_for_injections() {
        assert!(!needs_shell("plain", &["`code`".into()]));
        assert!(needs_shell("x", &["now: !`date`".into()]));
    }
}
