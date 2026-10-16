//! Small pre-dispatch checks for slash commands: closest-command hints, git preconditions and
//! catalog membership.

use super::*;

/// The note for an unknown `/x`: a path-like token gets the `//` escape hint, a near miss gets a
/// "did you mean".
pub(super) fn unknown_command_note(x: &str, line: &str) -> String {
    if x.contains('/') || x.contains('.') {
        return format!(
            "unknown command: /{x} — to send a path as a prompt, start the line with // (e.g. /{line})"
        );
    }
    let lower = x.to_lowercase();
    let near = forge_tui::COMMANDS
        .iter()
        .map(|c| (edit_distance(c.name, &lower), c.name))
        .filter(|(d, _)| *d <= 2 && *d < lower.len().max(1))
        .min_by_key(|(d, _)| *d);
    match near {
        Some((_, name)) => format!("unknown command: /{x} — did you mean /{name}? (try /help)"),
        None => format!("unknown command: /{x} — try /help"),
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Why `/commit` (`need_changes`) or `/pr` cannot do anything in `root`, if it cannot.
pub(super) fn git_blocker(root: &std::path::Path, need_changes: bool) -> Option<&'static str> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success());
    match out {
        None => Some("⚠ not a git repository — nothing to do here"),
        Some(o) if need_changes && o.stdout.iter().all(u8::is_ascii_whitespace) => {
            Some("⚠ no changes to commit — the working tree is clean")
        }
        Some(_) => None,
    }
}

/// Whether `/model <id>` named something the catalog does not list.
pub(super) async fn model_not_in_catalog(
    session: &Arc<tokio::sync::Mutex<Session>>,
    model_id: &str,
) -> bool {
    let s = session.lock().await;
    s.catalog().is_some_and(|c| {
        !c.by_provider(s.pricing())
            .iter()
            .any(|g| g.models.iter().any(|m| m.id == model_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_misses_and_paths_get_hints() {
        assert!(unknown_command_note("hepl", "/hepl").contains("did you mean /help"));
        assert!(unknown_command_note("etc/hosts", "/etc/hosts").contains("start the line with //"));
        assert!(!unknown_command_note("zzzzzz", "/zzzzzz").contains("did you mean"));
    }

    #[test]
    fn a_non_repo_blocks_commit_and_pr() {
        let dir = std::env::temp_dir().join(format!("forge-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(git_blocker(&dir, true).is_some());
        assert!(git_blocker(&dir, false).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
