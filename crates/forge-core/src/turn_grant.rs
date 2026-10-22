//! What an invoked skill or command changes for the turn it starts (Claude Code's skill
//! frontmatter, docs/features/command-skill-system.md):
//!
//! - `allowed-tools` become allow rules and `disallowed-tools` deny rules, tagged
//!   [`RuleSource::Turn`]. They sit beside the configured rules, so a configured deny, the
//!   built-in safety floor and `plan` mode still win over a skill's allow.
//! - `effort` sets the reasoning rung for the turn.
//! - `context: fork` runs the turn as a subagent (see `skill_fork.rs`).
//!
//! A grant is queued with [`Session::grant_next_turn`] and installed when `run_turn_with` starts.
//! It ends with that turn, as in Claude Code ("the grant clears when you send your next message"):
//! the rules are removed when the turn returns, and again at the start of the next turn in case
//! the previous one was interrupted mid-flight.

use forge_types::{EffortLevel, PermissionDecision, PermissionRule, RuleSource};

use super::*;

/// One turn's overrides from an invoked skill or command.
#[derive(Debug, Clone, Default)]
pub struct TurnGrant {
    /// Who asked for it (`skill pr-summary`), used in rule reasons.
    pub label: String,
    pub rules: Vec<PermissionRule>,
    pub effort: Option<EffortLevel>,
    pub fork: Option<ForkTurn>,
}

/// Run the turn as one subagent instead of the main loop (`context: fork`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkTurn {
    /// The agent type to run (`agent:`), `general-purpose` when the skill names none.
    pub agent: String,
    /// The skill's `model:`, which picks the subagent's model.
    pub model: Option<String>,
    /// The line recorded as the user's message (`/pr-summary 12`); the expanded skill text goes to
    /// the subagent, not into this session's context.
    pub label: String,
}

impl TurnGrant {
    /// The grant `invocation` declares. `allow_grants` is false for a definition from a project
    /// the user has not trusted: its `allowed-tools` would widen permissions, so they are dropped,
    /// while its `disallowed-tools` (which only narrow) still apply. `vars` expands
    /// `${CLAUDE_SKILL_DIR}`/`${CLAUDE_PROJECT_DIR}` inside rules, as Claude Code does.
    pub fn from_invocation(
        label: &str,
        invocation: &forge_skills::Invocation,
        allow_grants: bool,
        vars: &forge_skills::TemplateVars,
    ) -> TurnGrant {
        let mut rules = Vec::new();
        if allow_grants {
            let reason = format!("allowed-tools of {label}");
            for spec in &invocation.allowed_tools {
                rules.extend(forge_config::cc_rules::rules_for_spec(
                    &vars.apply(spec),
                    PermissionDecision::Allow,
                    RuleSource::Turn,
                    &reason,
                ));
            }
        }
        let reason = format!("disallowed-tools of {label}");
        for spec in &invocation.disallowed_tools {
            rules.extend(forge_config::cc_rules::rules_for_spec(
                &vars.apply(spec),
                PermissionDecision::Deny,
                RuleSource::Turn,
                &reason,
            ));
        }
        TurnGrant {
            label: label.to_string(),
            rules,
            effort: invocation
                .effort
                .as_deref()
                .and_then(forge_config::agents::parse_effort),
            fork: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.effort.is_none() && self.fork.is_none()
    }

    /// A one-line summary of the permission side of the grant, for the user (empty when none).
    pub fn describe(&self) -> String {
        let list = |decision: PermissionDecision| {
            self.rules
                .iter()
                .filter(|r| r.decision == decision)
                .map(|r| match r.patterns.as_slice() {
                    [] => r.tool.clone(),
                    patterns => format!("{}({})", r.tool, patterns.join(", ")),
                })
                .collect::<Vec<_>>()
        };
        let mut parts = Vec::new();
        let allowed = list(PermissionDecision::Allow);
        if !allowed.is_empty() {
            parts.push(format!("allowed this turn: {}", allowed.join(", ")));
        }
        let denied = list(PermissionDecision::Deny);
        if !denied.is_empty() {
            parts.push(format!("blocked this turn: {}", denied.join(", ")));
        }
        if let Some(effort) = self.effort {
            parts.push(format!("effort {}", effort.as_str()));
        }
        parts.join(" · ")
    }
}

/// The session's grant bookkeeping: the queued grant and the active turn's effort.
#[derive(Debug, Default)]
pub(crate) struct TurnGrantState {
    pending: Option<TurnGrant>,
    effort: Option<EffortLevel>,
}

impl Session {
    /// Queue `grant` for the next turn this session runs (replacing any grant still queued).
    pub fn grant_next_turn(&mut self, grant: TurnGrant) {
        self.turn_grant.pending = (!grant.is_empty()).then_some(grant);
    }

    /// Install the queued grant for the turn that is starting. Rules a previous turn left behind
    /// (it was interrupted before it could clean up) are removed first. Returns the fork, when the
    /// turn should run as a subagent.
    pub(crate) fn begin_turn_grant(&mut self) -> Option<ForkTurn> {
        self.end_turn_grant();
        let grant = self.turn_grant.pending.take()?;
        self.add_turn_rules(grant.rules);
        self.turn_grant.effort = grant.effort;
        grant.fork
    }

    /// Drop every turn-scoped rule and the turn's effort.
    pub(crate) fn end_turn_grant(&mut self) {
        self.rules.retain(|r| r.source != RuleSource::Turn);
        self.turn_grant.effort = None;
    }

    /// Add a grant that starts mid-turn (the model loaded a skill with `use_skill`). Its rules
    /// last until the turn ends; its effort cannot change a request already routed.
    pub(crate) fn extend_turn_grant(&mut self, grant: TurnGrant) {
        self.add_turn_rules(grant.rules);
    }

    /// Turn rules go in front of the configured ones: among equally specific allow/ask rules the
    /// broker takes the last, so a configured `ask` keeps outranking a skill's `allow`.
    fn add_turn_rules(&mut self, rules: Vec<PermissionRule>) {
        self.rules.splice(0..0, rules);
    }

    /// The reasoning rung the active turn's skill asked for, if any.
    pub(crate) fn turn_effort(&self) -> Option<EffortLevel> {
        self.turn_grant.effort
    }

    /// Whether a turn-scoped deny removes `tool` outright (no argument pattern), so it is left out
    /// of the tool surface the model sees — Claude Code's "removed from the available pool".
    pub(crate) fn turn_removes_tool(&self, tool: &str) -> bool {
        self.rules.iter().any(|r| {
            r.source == RuleSource::Turn
                && r.decision == PermissionDecision::Deny
                && r.patterns.is_empty()
                && r.tool == tool
        })
    }

    /// The rules the broker resolves against right now (built-in, configured, turn-scoped).
    pub fn permission_rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    /// Values for `${CLAUDE_SESSION_ID}`, `${CLAUDE_PROJECT_DIR}` and `${CLAUDE_EFFORT}` in skill
    /// and command text.
    pub fn skill_template_vars(&self) -> forge_skills::TemplateVars {
        let effort = self.exact_effort.or(self.pinned_effort).map(|e| match e {
            EffortLevel::WhiteHot => "max".to_string(),
            other => other.as_str().to_string(),
        });
        forge_skills::TemplateVars {
            session_id: Some(self.id.clone()),
            project_dir: Some(self.workspace.root().display().to_string()),
            skill_dir: None,
            effort,
        }
    }

    /// A skill the model asked for with `use_skill`: its rendered methodology and the grant its
    /// frontmatter carries. A project skill's `allowed-tools` count only in a trusted project.
    pub(crate) fn load_skill_for_model(&self, name: &str) -> Option<(String, TurnGrant)> {
        let catalog = self.skills.as_ref()?;
        let meta = catalog.model_skill(name, self.commands_trust_project())?;
        let vars = self.skill_template_vars();
        let guidance = forge_skills::Skill::load(meta).render(None, &vars);
        let allow_grants = meta.scope != forge_skills::Scope::Project
            || forge_config::project_trust::is_trusted(self.workspace.root());
        let grant = TurnGrant::from_invocation(
            &format!("skill {}", meta.name),
            &meta.invocation,
            allow_grants,
            &vars.for_skill_dir(&meta.dir),
        );
        Some((guidance, grant))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(
        allowed: &[&str],
        disallowed: &[&str],
        effort: Option<&str>,
    ) -> forge_skills::Invocation {
        forge_skills::Invocation {
            allowed_tools: allowed.iter().map(|s| s.to_string()).collect(),
            disallowed_tools: disallowed.iter().map(|s| s.to_string()).collect(),
            effort: effort.map(str::to_string),
            ..forge_skills::Invocation::default()
        }
    }

    #[test]
    fn grant_maps_tools_rules_and_effort() {
        let vars = forge_skills::TemplateVars {
            skill_dir: Some("/skills/chart".into()),
            ..Default::default()
        };
        let grant = TurnGrant::from_invocation(
            "skill chart",
            &invocation(
                &["Bash(${CLAUDE_SKILL_DIR}/render.sh *)", "Read"],
                &["AskUserQuestion"],
                Some("max"),
            ),
            true,
            &vars,
        );
        assert_eq!(grant.rules.len(), 3);
        assert!(grant.rules.iter().all(|r| r.source == RuleSource::Turn));
        assert_eq!(grant.rules[0].tool, "shell");
        assert_eq!(grant.rules[0].patterns, ["/skills/chart/render.sh *"]);
        assert_eq!(grant.rules[2].decision, PermissionDecision::Deny);
        assert_eq!(grant.rules[2].tool, "ask_user");
        assert_eq!(grant.effort, Some(EffortLevel::XHigh));
        assert_eq!(
            grant.describe(),
            "allowed this turn: shell(/skills/chart/render.sh *), read_file · blocked this turn: ask_user · effort xhigh"
        );
    }

    #[test]
    fn untrusted_definitions_keep_only_restrictions() {
        let grant = TurnGrant::from_invocation(
            "skill x",
            &invocation(&["Bash"], &["Write"], None),
            false,
            &Default::default(),
        );
        assert!(grant
            .rules
            .iter()
            .all(|r| r.decision == PermissionDecision::Deny));
        assert!(!grant.rules.is_empty());
        assert!(TurnGrant::from_invocation(
            "x",
            &invocation(&[], &[], None),
            true,
            &Default::default()
        )
        .is_empty());
    }
}
