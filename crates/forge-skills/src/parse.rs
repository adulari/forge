//! Reading one command file or `SKILL.md` into its catalog entry.

use std::path::Path;

use crate::frontmatter::{self, Frontmatter};
use crate::{parse_tier, ArgSpec, Command, Invocation, Scope, SkillMeta};

pub(crate) fn parse_command(
    raw: &str,
    stem: &str,
    scope: Scope,
    path: &Path,
) -> Result<Command, String> {
    let (fm_text, body) = frontmatter::split(raw);
    let fm = frontmatter::parse(fm_text.unwrap_or(""))?;
    let body = body.trim().to_string();
    if body.is_empty() {
        return Err("empty command body".into());
    }
    let name = fm.scalar("name").unwrap_or_else(|| stem.to_string());
    let description = fm
        .scalar("description")
        .unwrap_or_else(|| first_line(&body, 60));
    // Forge's `args: [a, b?]` declares required args; Claude Code's `arguments:` names optional
    // ones (a missing value expands to nothing).
    let mut args: Vec<ArgSpec> = fm
        .list("args")
        .into_iter()
        .map(|a| {
            let required = !a.ends_with('?');
            ArgSpec {
                name: a.trim_end_matches('?').to_string(),
                required,
            }
        })
        .collect();
    if args.is_empty() {
        args = claude_arguments(&fm)
            .into_iter()
            .map(|name| ArgSpec {
                name,
                required: false,
            })
            .collect();
    }
    let tier = fm.scalar("tier").and_then(|t| parse_tier(&t));
    let model = fm.scalar("model");
    Ok(Command {
        name,
        description,
        args,
        tier,
        model,
        body,
        scope,
        path: path.to_path_buf(),
        invocation: Invocation::from_frontmatter(&fm),
    })
}

pub(crate) fn parse_skill_meta(
    raw: &str,
    stem: &str,
    scope: Scope,
    dir: &Path,
) -> Result<SkillMeta, String> {
    let (fm_text, body) = frontmatter::split(raw);
    let fm = frontmatter::parse(fm_text.unwrap_or(""))?;
    let name = fm.scalar("name").unwrap_or_else(|| stem.to_string());
    let description = fm
        .scalar("description")
        .unwrap_or_else(|| first_line(body.trim(), 60));
    let tier = fm.scalar("tier").and_then(|t| parse_tier(&t));
    let resources = fm.list("resources");
    Ok(SkillMeta {
        name,
        description,
        tier,
        resources,
        dir: dir.to_path_buf(),
        scope,
        body: None,
        model: fm.scalar("model"),
        arg_names: claude_arguments(&fm),
        invocation: Invocation::from_frontmatter(&fm),
    })
}

/// Claude Code's `arguments:` — a YAML list or one space-separated string of names.
pub(crate) fn claude_arguments(fm: &Frontmatter) -> Vec<String> {
    fm.list("arguments")
        .iter()
        .flat_map(|a| a.split_whitespace())
        .map(str::to_string)
        .collect()
}

pub(crate) fn first_line(body: &str, max: usize) -> String {
    let line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.chars().count() > max {
        line.chars().take(max).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}
