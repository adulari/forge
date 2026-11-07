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

/// The `scout` tool's agent (docs/features/scouts.md): read-only, cheap tier, and bound to hand
/// back a short cited brief, because that brief is all the parent ever sees of the run.
const SCOUT_AGENT: &str = "scout";

const SCOUT_PROMPT: &str = "You are a scout: a throwaway, read-only codebase explorer. A parent \
agent will see ONLY your final answer, never your reads, so everything it needs must be in it. \
Answer the question you were given by locating the relevant files and reading just the parts that \
matter: prefer `search`, `glob` and `lattice` over reading whole files, and read line ranges. \
You cannot edit anything; shell is limited to read-only commands. Stop as soon as the question is \
answered. Your final answer is a brief of at most 400 words (about 2k tokens) in this shape: \
one line stating the answer; then a bullet list of facts, each ending with a citation \
`path:start-end`; then, if useful, `Not found:` listing what you looked for and did not find. \
Never paste long code excerpts: cite the lines instead, and quote at most a line or two when the \
exact text matters. Every claim needs a citation; say plainly when something does not exist.";

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
            name: SCOUT_AGENT.into(),
            description: "Throwaway orientation run behind the `scout` tool: read-only, cheap \
                          tier, returns a short brief with path:line citations."
                .into(),
            tools: READ_ONLY_TOOLS
                .iter()
                .chain(&["lattice"])
                .map(|t| t.to_string())
                .collect(),
            tier: Some(TaskTier::Trivial),
            system_prompt: SCOUT_PROMPT.into(),
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
        assert_eq!(agents.len(), 4);
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
