//! Agent CLI install state for `forge doctor`, and the `--fix` path that updates what is outdated.

use forge_provider::agent_install::{self, AgentStatus};
use forge_provider::CliKind;

use crate::doctor::{check, Check, Status};

fn runtime_suffix(runtime: &str) -> &'static str {
    match runtime {
        "wsl" => " · WSL",
        "windows_interop" => " · Windows (via WSL)",
        _ => "",
    }
}

/// One row per bridge CLI: installed-vs-latest, never an error (every bridge is optional).
pub(crate) fn rows(statuses: &[AgentStatus]) -> Vec<Check> {
    statuses
        .iter()
        .map(|s| {
            let label = format!("{} cli", s.id.trim_end_matches("-cli"));
            let suffix = runtime_suffix(s.runtime);
            if !s.installed {
                return check(
                    Status::Info,
                    &label,
                    "not installed",
                    s.command
                        .as_deref()
                        .map(|c| {
                            format!("`forge doctor --fix` runs `{c}` (or in the app: Providers)")
                        })
                        .or_else(|| s.manual_hint.clone())
                        .as_deref(),
                );
            }
            let version = s.version.as_deref().unwrap_or("version unknown");
            match (&s.latest, s.outdated) {
                (Some(latest), true) => check(
                    Status::Warn,
                    &label,
                    format!("{version}{suffix}; {latest} available"),
                    Some("run `forge doctor --fix` to update"),
                ),
                (Some(_), false) => check(
                    Status::Ok,
                    &label,
                    format!("{version}{suffix} (latest)"),
                    None,
                ),
                (None, _) => check(Status::Ok, &label, format!("{version}{suffix}"), None),
            }
        })
        .collect()
}

pub(crate) async fn section() -> Vec<Check> {
    rows(&agent_install::statuses(true).await)
}

/// Update every installed-and-outdated bridge CLI, printing each vendor command's output as it
/// runs. Missing CLIs are left alone: they are optional, and installing one is a choice.
/// Returns the number of updates that failed.
pub(crate) async fn fix() -> usize {
    let statuses = agent_install::statuses(true).await;
    let unchecked: Vec<(&str, &str)> = statuses
        .iter()
        .filter(|s| s.installed && s.latest.is_none() && s.id != "agy-cli")
        .map(|s| (s.id, s.name))
        .collect();
    for (_, name) in &unchecked {
        println!(
            "  · {}: could not check the latest version (offline?), skipped",
            name
        );
    }
    let unchecked = unchecked.len();
    let outdated: Vec<CliKind> = statuses
        .into_iter()
        .filter(|s| s.outdated)
        .filter_map(|s| CliKind::all().into_iter().find(|k| k.prefix() == s.id))
        .collect();
    if outdated.is_empty() {
        if unchecked == 0 {
            println!("  · agent CLIs: nothing to update\n");
        } else {
            println!();
        }
        return 0;
    }
    let mut failed = 0;
    for kind in outdated {
        println!("  updating {} …", agent_install::display_name(kind));
        let result = agent_install::apply(kind, |line| println!("      {line}")).await;
        match result {
            Ok(done) => println!(
                "  ✓ {} {} -> {}\n",
                agent_install::display_name(kind),
                done.before.as_deref().unwrap_or("?"),
                done.after.as_deref().unwrap_or("?"),
            ),
            Err(error) => {
                failed += 1;
                println!("  ✗ {}: {error}\n", agent_install::display_name(kind));
            }
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_provider::agent_install::Action;

    fn status(installed: bool, version: Option<&str>, latest: Option<&str>) -> AgentStatus {
        let outdated = match (version, latest) {
            (Some(v), Some(l)) => agent_install::is_older(v, l),
            _ => false,
        };
        AgentStatus {
            id: "claude-cli",
            name: "Claude Code",
            installed,
            version: version.map(str::to_string),
            latest: latest.map(str::to_string),
            outdated,
            runtime: "native",
            action: Some(if installed {
                Action::Update
            } else {
                Action::Install
            }),
            command: Some("claude update".into()),
            manual_hint: None,
        }
    }

    #[test]
    fn an_outdated_cli_warns_and_points_at_fix() {
        let rows = rows(&[status(true, Some("2.0.0"), Some("2.1.0"))]);
        assert_eq!(rows[0].status, Status::Warn);
        assert!(rows[0].detail.contains("2.1.0 available"));
        assert!(rows[0].fix.as_deref().unwrap().contains("doctor --fix"));
    }

    #[test]
    fn current_and_missing_clis_never_fail_the_doctor() {
        let current = rows(&[status(true, Some("2.1.0"), Some("2.1.0"))]);
        assert_eq!(current[0].status, Status::Ok);
        let missing = rows(&[status(false, None, None)]);
        assert_eq!(missing[0].status, Status::Info);
    }
}
