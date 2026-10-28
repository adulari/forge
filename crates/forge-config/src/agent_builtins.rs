//! The subagent types Forge ships, so Claude Code's `Explore` and `Plan` work without a file.
//! A user or project agent file with the same name replaces one (they are the lowest layer of
//! [`crate::load_agents_layered`]).

use std::collections::HashMap;

use forge_types::{PermissionMode, TaskTier};

use crate::AgentDef;

/// Read-only tools for the built-ins. `shell` is safe here because both run in `plan` mode,
/// where only commands the read-only allowlist recognises (`git log`, `ls`, `rg`) may run.
const READ_ONLY_TOOLS: &[&str] = &["read_file", "list_dir", "search", "glob", "shell"];

const EXPLORE_PROMPT: &str = "You are a fast, read-only codebase explorer working for a parent \
agent. Find what was asked: locate files, search for symbols and strings, read the relevant \
parts, and trace how things connect. You cannot edit anything; shell is limited to read-only \
commands such as `git log`, `git grep`, `ls` and `rg`. Be quick: prefer targeted searches over \
reading whole files. Answer with concrete findings and `path:line` references, and say plainly \
when something does not exist.";

const PLAN_PROMPT: &str = "You are a read-only planning researcher working for a parent agent. \
Gather the context a change needs before anyone edits: the files and functions involved, their \
callers and tests, conventions the change must follow, and the risks. You cannot edit anything; \
shell is limited to read-only commands. Return a concise implementation plan: the steps in \
order, the exact files to touch with `path:line` references, how to verify the result, and any \
open questions the parent must decide.";

const GENERAL_PURPOSE_PROMPT: &str =
    "You are a general-purpose Forge subagent working for a parent agent \
on ONE delegated task. Do it end to end with the tools you have: search and read code, edit and \
create files, run commands and tests. Prefer editing existing files to creating new ones, and \
touch only what the task needs. Your tools run under the session's permission mode; if a call is \
refused, say so in your answer rather than working around it. Finish with a concise report of \
what you found or changed (files and `path:line` references), written for the parent, no preamble.";

pub(crate) fn builtin_agents() -> HashMap<String, AgentDef> {
    let tools = || READ_ONLY_TOOLS.iter().map(|t| t.to_string()).collect();
    [
        AgentDef {
            name: "general-purpose".into(),
            description: "General-purpose agent for researching, searching and carrying out \
                          multi-step tasks, including editing files and running commands under \
                          the session's permission mode."
                .into(),
            system_prompt: GENERAL_PURPOSE_PROMPT.into(),
            ..AgentDef::default()
        },
        AgentDef {
            name: "explore".into(),
            description: "Fast read-only codebase search: find files, symbols and how code \
                          connects; answers with path:line references."
                .into(),
            tools: tools(),
            tier: Some(TaskTier::Trivial),
            system_prompt: EXPLORE_PROMPT.into(),
            permission_mode: Some(PermissionMode::Plan),
            ..AgentDef::default()
        },
        AgentDef {
            name: "plan".into(),
            description: "Read-only research before a change: gathers files, callers, tests \
                          and risks, returns a step-by-step implementation plan."
                .into(),
            tools: tools(),
            system_prompt: PLAN_PROMPT.into(),
            permission_mode: Some(PermissionMode::Plan),
            ..AgentDef::default()
        },
    ]
    .into_iter()
    .map(|def| (def.name.clone(), def))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explore_and_plan_are_read_only_and_unsourced() {
        let agents = builtin_agents();
        assert_eq!(agents.len(), 3);
        for name in ["explore", "plan"] {
            let def = &agents[name];
            assert_eq!(def.permission_mode, Some(PermissionMode::Plan));
            assert!(def.path.is_none());
            assert!(!def
                .tools
                .iter()
                .any(|t| t == "write_file" || t == "edit_file"));
        }
        assert_eq!(agents["explore"].tier, Some(TaskTier::Trivial));
        assert_eq!(agents["plan"].tier, None);
    }

    #[test]
    fn general_purpose_has_every_tool_and_the_parents_temper() {
        let def = &builtin_agents()["general-purpose"];
        assert!(def.tools.is_empty());
        assert!(def.disallowed_tools.is_empty());
        assert_eq!(def.permission_mode, None);
        assert!(def.path.is_none());
    }
}
