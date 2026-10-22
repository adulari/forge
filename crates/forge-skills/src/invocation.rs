//! Claude Code's invocation frontmatter for skills and commands: who may invoke a definition, what
//! it may do while it runs, and where it runs. Parsed once at discovery so the catalog, the
//! dispatcher and the turn loop all read the same values.
//!
//! | key | effect in Forge |
//! | --- | --- |
//! | `allowed-tools` | allow rules for the invoking turn only (see `forge_core::turn_grant`) |
//! | `disallowed-tools` | deny rules for the invoking turn only |
//! | `disable-model-invocation` | hidden from the `use_skill` listing; the model cannot load it |
//! | `user-invocable: false` | hidden from the `/` palette; typing `/name` is refused |
//! | `argument-hint` | the palette's argument hint |
//! | `context: fork` + `agent` | the invocation runs as one subagent of that type |
//! | `paths` | advertised next to the skill so the model loads it for matching files |
//! | `effort` | the reasoning rung for the invoking turn |

use crate::frontmatter::Frontmatter;

/// Where an invocation runs: `context: fork` hands it to a subagent instead of the main loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForkSpec {
    /// The `agent:` type to run it as (`None` = Forge's general read-only subagent).
    pub agent: Option<String>,
}

/// The invocation policy a skill or command declares. `Default` is "no restrictions, no grants".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    /// `false` when `disable-model-invocation: true`: only the user can run it with `/name`.
    pub model_invocable: bool,
    /// `false` when `user-invocable: false`: only the model can load it.
    pub user_invocable: bool,
    pub argument_hint: Option<String>,
    pub fork: Option<ForkSpec>,
    pub paths: Vec<String>,
    /// The raw `effort:` value (`low` … `max`); mapped onto Forge's rungs by the caller.
    pub effort: Option<String>,
}

impl Default for Invocation {
    fn default() -> Self {
        Self {
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            model_invocable: true,
            user_invocable: true,
            argument_hint: None,
            fork: None,
            paths: Vec::new(),
            effort: None,
        }
    }
}

impl Invocation {
    pub fn from_frontmatter(fm: &Frontmatter) -> Self {
        let fork = fm
            .scalar("context")
            .filter(|c| c.trim().eq_ignore_ascii_case("fork"))
            .map(|_| ForkSpec {
                agent: fm.scalar("agent").map(|a| a.trim().to_string()),
            });
        Self {
            allowed_tools: fm.tool_list("allowed-tools"),
            disallowed_tools: fm.tool_list("disallowed-tools"),
            model_invocable: !fm.flag("disable-model-invocation").unwrap_or(false),
            user_invocable: fm.flag("user-invocable").unwrap_or(true),
            argument_hint: argument_hint(fm),
            fork,
            paths: fm
                .list("paths")
                .iter()
                .flat_map(|p| p.split(','))
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect(),
            effort: fm.scalar("effort").map(|e| e.trim().to_ascii_lowercase()),
        }
    }

    /// Whether running this definition changes anything beyond the prompt: a permission grant or
    /// restriction, an effort rung, or a fork.
    pub fn has_turn_effects(&self) -> bool {
        !self.allowed_tools.is_empty()
            || !self.disallowed_tools.is_empty()
            || self.effort.is_some()
            || self.fork.is_some()
    }
}

/// `argument-hint: [issue-number]` is a YAML flow list, which is how Claude Code's own docs write
/// it; read it back as the bracketed text the user wrote.
fn argument_hint(fm: &Frontmatter) -> Option<String> {
    fm.scalar("argument-hint").or_else(|| {
        let items = fm.list("argument-hint");
        (!items.is_empty()).then(|| format!("[{}]", items.join(", ")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::parse;

    #[test]
    fn claude_code_fields_parse() {
        let fm = parse(concat!(
            "allowed-tools: Bash(gh *) Read\n",
            "disallowed-tools: [AskUserQuestion]\n",
            "disable-model-invocation: yes\n",
            "user-invocable: false\n",
            "argument-hint: [issue-number]\n",
            "context: fork\n",
            "agent: Explore\n",
            "paths: src/**/*.rs, docs/*.md\n",
            "effort: HIGH\n",
        ))
        .unwrap();
        let inv = Invocation::from_frontmatter(&fm);
        assert_eq!(inv.allowed_tools, ["Bash(gh *)", "Read"]);
        assert_eq!(inv.disallowed_tools, ["AskUserQuestion"]);
        assert!(!inv.model_invocable);
        assert!(!inv.user_invocable);
        // A bracketed hint is a YAML inline list to the reader; it still reads back whole.
        assert_eq!(inv.argument_hint.as_deref(), Some("[issue-number]"));
        assert_eq!(
            inv.fork,
            Some(ForkSpec {
                agent: Some("Explore".into())
            })
        );
        assert_eq!(inv.paths, ["src/**/*.rs", "docs/*.md"]);
        assert_eq!(inv.effort.as_deref(), Some("high"));
        assert!(inv.has_turn_effects());
    }

    #[test]
    fn absent_fields_mean_no_restrictions() {
        let inv = Invocation::from_frontmatter(&parse("description: x\n").unwrap());
        assert_eq!(inv, Invocation::default());
        assert!(inv.model_invocable && inv.user_invocable);
        assert!(!inv.has_turn_effects());
    }

    #[test]
    fn context_other_than_fork_runs_inline() {
        let inv = Invocation::from_frontmatter(&parse("context: inline\nagent: x\n").unwrap());
        assert_eq!(inv.fork, None);
    }
}
