//! The catalog Forge ships inside the binary.
//!
//! These entries need no filesystem: they exist on a fresh install with empty user and project
//! scopes, and a user/project definition of the same name always overrides them (see
//! `Catalog::insert_command` / `insert_builtin_skills`). Their prose IS the behaviour — the
//! `/orchestrate` body and the standing guidance below are what the model actually reads — so it
//! lives here rather than being scattered through catalog assembly.

use std::path::PathBuf;

use forge_types::TaskTier;

use crate::{frontmatter, parse_tier, Command, Scope, SkillMeta};

/// The skills compiled into the binary. Each carries its methodology inline (`body: Some`) so it
/// loads with no filesystem access — available even on a fresh install with empty user/project
/// scopes. The SKILL.md is the single source of truth for name/description/tier; a malformed one
/// is skipped rather than panicking at startup.
pub(crate) fn builtin_skills() -> Vec<SkillMeta> {
    const RUST_BEST_PRACTICES: &str = include_str!("../builtin/rust-best-practices/SKILL.md");
    [("rust-best-practices", RUST_BEST_PRACTICES)]
        .into_iter()
        .filter_map(|(name, raw)| parse_builtin_skill(name, raw))
        .collect()
}

/// Parse a compiled-in SKILL.md into a [`SkillMeta`] whose body is held inline. Unlike
/// [`parse_skill_meta`], the body travels with the metadata (there is no `dir` to re-read at use
/// time). Returns `None` for a SKILL.md with unparsable frontmatter or no description.
fn parse_builtin_skill(name: &str, raw: &str) -> Option<SkillMeta> {
    let (fm_text, body) = frontmatter::split(raw);
    let fm = frontmatter::parse(fm_text.unwrap_or("")).ok()?;
    let description = fm.scalar("description")?;
    Some(SkillMeta {
        name: fm.scalar("name").unwrap_or_else(|| name.to_string()),
        description,
        tier: fm.scalar("tier").and_then(|t| parse_tier(&t)),
        resources: Vec::new(),
        dir: PathBuf::from(format!("<builtin>/skills/{name}")),
        scope: Scope::Builtin,
        body: Some(body.trim().to_string()),
    })
}

/// Build the built-in `/rust` command: a thin wrapper that loads the `rust-best-practices` skill.
/// The `**rust-best-practices**` reference in the body is resolved to the skill's full methodology
/// by [`Catalog::referenced_skill_guidance`] at invoke time. A user/project command named `rust`
/// overrides it.
pub(crate) fn builtin_rust_command() -> Command {
    Command {
        name: "rust".to_string(),
        description: "Apply Rust best practices — idiomatic error handling, ownership, API design, clippy/fmt, and size limits — to the Rust you write or review.".to_string(),
        args: Vec::new(),
        tier: Some(TaskTier::Standard),
        model: None,
        body: concat!(
            "Apply the **rust-best-practices** skill to this task. Follow its methodology for any\n",
            "Rust code you write, review, or refactor, and run the verification gate (fmt, clippy,\n",
            "test) before reporting done.\n\n",
            "Task: $ARGUMENTS"
        )
        .to_string(),
        scope: Scope::Builtin,
        path: PathBuf::from("<builtin>/commands/rust.md"),
    }
}

/// Build the built-in `/orchestrate` command. User/project commands or skills named `orchestrate`
/// take precedence over this builtin.
pub(crate) fn builtin_orchestrate_command() -> Command {
    Command {
        name: "orchestrate".to_string(),
        description: "Route a task through the best available Forge resources — skills, subagents, MCP, web, Lattice, or direct implementation.".to_string(),
        args: Vec::new(),
        tier: Some(TaskTier::Complex),
        model: None,
        body: concat!(
            "Orchestrate this Forge task. Check ALL resource categories before deciding; ",
            "never skip one that fits.\n\n",
            "Task: $ARGUMENTS\n\n",
            "RESOURCE DECISION ORDER:\n\n",
            "1. Skills (always first) — read the available skills already listed in the `use_skill`\n",
            "   tool description. A skill is a tested, project-aware methodology. If any skill covers this task\n",
            "   (fully or partially) → invoke it via `use_skill <name>`. Don't implement from\n",
            "   scratch what a skill already does well.\n\n",
            "2. Subagents — use `spawn_agents` when 2+ subtasks are genuinely independent\n",
            "   (can run in parallel). Not for sequential steps — those run in one turn.\n\n",
            "3. MCP tools — check the live tool list for the right integration (GitHub, search,\n",
            "   databases, calendars, etc.). Prefer the correct MCP tool over a shell workaround.\n\n",
            "4. Web — use `web_search` or `web_fetch` for current docs, package versions, or\n",
            "   any information that isn't in the project files.\n\n",
            "5. Code intelligence — use `lattice_query` for symbol lookups and cross-file\n",
            "   navigation (\"where is X defined\", \"what calls Y\"). More precise than grep.\n\n",
            "6. Shell / file tools — direct edits, builds, tests — when no higher-level tool\n",
            "   applies.\n\n",
            "7. `ask_user` — only when a decision cannot be inferred and is genuinely the\n",
            "   user's to make. One focused question; never a list.\n\n",
            "RULES:\n",
            "• Highest-level tool wins: skill > bare implementation, MCP > shell, subagent > sequential.\n",
            "• Compose freely: a task may use a skill + MCP tool + shell together.\n",
            "• State a one-sentence plan and execute — ask only when the task is ambiguous,\n",
            "  destructive, or requires a decision only the user can make. Don't stall."
        )
        .to_string(),
        scope: Scope::Builtin,
        path: PathBuf::from("<builtin>/commands/orchestrate.md"),
    }
}

/// Standing orchestration guidance injected once per session when `mesh.auto_orchestrate = true`.
/// Compact so it doesn't bloat context — the full decision tree is in the `/orchestrate` command.
pub fn orchestrate_system_guidance() -> &'static str {
    concat!(
        "Forge auto-orchestrate is active. Consider available resource categories before executing; \
use only those that materially reduce work:\n\n",
        "1. Skills first — read the available skills in the `use_skill` tool description. Use a\n",
        "   matching skill rather than implementing from scratch; invoke only its exact name.\n",
        "2. Subagents (`spawn_agents`) only for 2+ independently useful deliverables. A single bug,\n",
        "   feature, or refactor remains ONE task even when it spans files. Do NOT delegate routine\n",
        "   repository exploration, code search, test discovery, or review for one task; direct\n",
        "   tools share context and are faster.\n",
        "3. MCP tools for external integrations — prefer the correct tool over shell workarounds.\n",
        "4. Web (`web_search`/`web_fetch`) for current information not in the project.\n",
        "5. Code intelligence (`lattice_query`) for symbol lookups and cross-file navigation.\n",
        "6. `ask_user` only when the decision is genuinely the user's — one question, not a list.\n\n",
        "Rule: use the highest-level tool that fits. Compose freely. Execute without asking unless\n",
        "the task is ambiguous, destructive, or requires a decision only the user can make."
    )
}
