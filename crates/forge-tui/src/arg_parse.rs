//! Raw-argument parsing for slash commands whose arg strings carry flags: `/assay` scopes and
//! the `/loop`/`/goal` autonomy options. Split from `commands.rs` so the command registry file
//! stays within its architecture-size budget.

use crate::commands::CommandAction;

/// Extract a comma-separated lens list from `--flag <value>` in a raw arg string.
/// `/assay --only dead-weight,unsafe` → `extract_flag(arg, "--only")` → `["dead-weight", "unsafe"]`
pub(crate) fn extract_flag(arg: &str, flag: &str) -> Vec<String> {
    let tokens: Vec<&str> = arg.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        if *tok == flag {
            if let Some(val) = tokens.get(i + 1) {
                if !val.starts_with('-') {
                    return val
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

/// Check whether a boolean flag (no value) is present in `arg`.
pub(crate) fn has_flag(arg: &str, flag: &str) -> bool {
    arg.split_whitespace().any(|t| t == flag)
}

/// `/assay [--diff|--branch <b>|--since <ref>|<path>] [--only <lens,…>] [--skip <lens,…>]`
pub(crate) fn assay_action(arg: &str) -> CommandAction {
    let only = extract_flag(arg, "--only");
    let skip = extract_flag(arg, "--skip");
    // Scope: --diff, --branch <b>, --since <ref>, a path, or empty (full repo).
    let scope = if has_flag(arg, "--diff") {
        "--diff".to_string()
    } else if let Some(b) = extract_flag(arg, "--branch").into_iter().next() {
        format!("--branch {b}")
    } else if let Some(r) = extract_flag(arg, "--since").into_iter().next() {
        format!("--since {r}")
    } else {
        // Remaining tokens that aren't flags → treat as path.
        arg.split_whitespace()
            .filter(|t| !t.starts_with("--"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    CommandAction::Assay { only, skip, scope }
}

/// `/goal <objective> [--gate "<cmd>"]... [--max-tokens N] [--max-minutes N]`
pub(crate) fn goal_action(arg: &str) -> CommandAction {
    let (objective, gates, max_tokens, max_minutes) = parse_autonomy_options(arg);
    CommandAction::Goal {
        objective,
        gates,
        max_tokens,
        max_minutes,
    }
}

/// `/loop <task> [--gate "<cmd>"]... [--max-tokens N] [--max-minutes N]`
pub(crate) fn loop_action(arg: &str) -> CommandAction {
    let (prompt, gates, max_tokens, max_minutes) = parse_autonomy_options(arg);
    CommandAction::Loop {
        prompt,
        gates,
        max_tokens,
        max_minutes,
    }
}

/// Parse the `/loop`/`/goal`-shared autonomous-gate/budget options out of a raw arg string:
/// `[--gate "<cmd>"]... [--max-tokens N] [--max-minutes N] <prompt>` in any order, returning
/// `(prompt, gates, max_tokens, max_minutes)`. `--gate` is repeatable; its value is quote-aware
/// (via `shell_words`) so a gate command containing spaces survives (`--gate "cargo test"`).
///
/// The common bare case (no option flags at all) returns `arg` completely untouched — no
/// tokenizing, no re-joining — so `/loop <task>`/`/goal <objective>` keep their exact prior
/// behavior (whitespace and all) for backward compatibility. Malformed quoting in the slow path
/// (unbalanced `"`) falls back the same way rather than erroring.
fn parse_autonomy_options(arg: &str) -> (String, Vec<String>, Option<u64>, Option<u64>) {
    let has_options =
        arg.contains("--gate") || arg.contains("--max-tokens") || arg.contains("--max-minutes");
    if !has_options {
        return (arg.to_string(), Vec::new(), None, None);
    }
    let Ok(tokens) = shell_words::split(arg) else {
        return (arg.to_string(), Vec::new(), None, None);
    };
    let mut gates = Vec::new();
    let mut max_tokens = None;
    let mut max_minutes = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--gate" if i + 1 < tokens.len() => {
                gates.push(tokens[i + 1].clone());
                i += 2;
            }
            "--max-tokens" if i + 1 < tokens.len() => {
                max_tokens = tokens[i + 1].parse().ok();
                i += 2;
            }
            "--max-minutes" if i + 1 < tokens.len() => {
                max_minutes = tokens[i + 1].parse().ok();
                i += 2;
            }
            _ => {
                rest.push(tokens[i].clone());
                i += 1;
            }
        }
    }
    (rest.join(" "), gates, max_tokens, max_minutes)
}
