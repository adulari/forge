//! `--tasks` / `--skip` for `forge bench tasks`: pick suite tasks by name or glob.

use anyhow::Result;

use super::suite::SuiteTask;

/// `*` and `?` match any run of characters / one character; everything else is literal.
fn glob_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                backtrack = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match backtrack {
                Some((star, matched)) => {
                    p = star + 1;
                    n = matched + 1;
                    backtrack = Some((star, matched + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

fn matching<'a>(patterns: &'a [String], tasks: &[SuiteTask]) -> Result<Vec<&'a str>> {
    let mut hits = Vec::new();
    for pattern in patterns {
        if !tasks.iter().any(|t| glob_matches(pattern, &t.name)) {
            anyhow::bail!(
                "no task matches '{pattern}' (available: {})",
                tasks
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        hits.push(pattern.as_str());
    }
    Ok(hits)
}

/// Keep the tasks named by `include` (every task when empty), then drop those named by `skip`.
/// A pattern that matches nothing is an error: a misspelt name must not silently run less.
pub(super) fn select_tasks(
    tasks: Vec<SuiteTask>,
    include: &[String],
    skip: &[String],
) -> Result<Vec<SuiteTask>> {
    let include = matching(include, &tasks)?;
    let skip = matching(skip, &tasks)?;
    let selected: Vec<SuiteTask> = tasks
        .into_iter()
        .filter(|t| include.is_empty() || include.iter().any(|p| glob_matches(p, &t.name)))
        .filter(|t| !skip.iter().any(|p| glob_matches(p, &t.name)))
        .collect();
    if selected.is_empty() {
        anyhow::bail!("--tasks/--skip left no task to run");
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn task(name: &str) -> SuiteTask {
        SuiteTask {
            name: name.into(),
            dir: PathBuf::from(name),
            instruction: String::new(),
            setup: None,
            test: PathBuf::from("test.sh"),
        }
    }

    fn names(tasks: &[SuiteTask]) -> Vec<&str> {
        tasks.iter().map(|t| t.name.as_str()).collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn suite() -> Vec<SuiteTask> {
        ["a01-fix", "a02-limiter", "b01-parse", "b02-fix"]
            .into_iter()
            .map(task)
            .collect()
    }

    #[test]
    fn glob_handles_star_question_and_literals() {
        assert!(glob_matches("a*", "a01-fix"));
        assert!(glob_matches("*-fix", "b02-fix"));
        assert!(glob_matches("a0?-*", "a02-limiter"));
        assert!(glob_matches("*", ""));
        assert!(!glob_matches("a?", "a01"));
        assert!(!glob_matches("fix", "a01-fix"));
        assert!(glob_matches("a*b*c", "aXbYbZc"));
    }

    #[test]
    fn names_and_globs_select_in_suite_order_and_skip_removes() {
        let picked = select_tasks(suite(), &strings(&["b02-fix", "a*"]), &[]).unwrap();
        assert_eq!(names(&picked), ["a01-fix", "a02-limiter", "b02-fix"]);
        let picked = select_tasks(suite(), &[], &strings(&["*-fix"])).unwrap();
        assert_eq!(names(&picked), ["a02-limiter", "b01-parse"]);
        let picked = select_tasks(suite(), &strings(&["a*"]), &strings(&["a02*"])).unwrap();
        assert_eq!(names(&picked), ["a01-fix"]);
    }

    #[test]
    fn a_pattern_matching_nothing_is_an_error_naming_the_choices() {
        let err = select_tasks(suite(), &strings(&["a01-fxi"]), &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("no task matches 'a01-fxi'"), "{err}");
        assert!(err.contains("b01-parse"), "{err}");
        assert!(select_tasks(suite(), &[], &strings(&["zzz"])).is_err());
    }

    #[test]
    fn skipping_everything_is_an_error() {
        assert!(select_tasks(suite(), &[], &strings(&["*"])).is_err());
    }
}
