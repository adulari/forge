//! Where a recursive delete may reach: the session workspace and the temp roots, nowhere else.
//!
//! `catastrophic.rs` refuses the home directory itself, the root and system trees. That leaves
//! `rm -rf ~/Documents` and `rm -rf /home/u/other-project` runnable, so a recursive delete is
//! also refused (in every temper, `bypass` included) when an absolute or `~` target is not
//! strictly inside the workspace or a temp directory, or when a relative target climbs out of the
//! workspace with `..`. An unknown workspace falls back to the process directory, never to
//! "anything goes".

use std::path::{Path, PathBuf};

use super::auto::{outside_workspace, split_words, strip_wrappers};
use super::{effective_commands, lexical_clean};

fn temp_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/var/tmp"),
        std::env::temp_dir(),
    ]
}

fn expand_home(operand: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if operand == "~" {
        return home;
    }
    match operand.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home.trim_end_matches('/')),
        None => operand.to_string(),
    }
}

/// Strictly below a temp root (`/tmp` itself and a bare wildcard directly under it do not
/// count), with no `..` escape or leaving symlink.
fn inside_temp(clean: &str) -> bool {
    temp_roots().iter().any(|root| {
        let root = root.to_string_lossy();
        let root = root.trim_end_matches('/');
        clean
            .strip_prefix(root)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|rest| {
                let first = rest.split('/').next().unwrap_or("");
                let literal = first
                    .chars()
                    .take_while(|c| !matches!(c, '*' | '?' | '['))
                    .count();
                (literal == first.chars().count() && literal > 0) || literal >= 3
            })
    }) && match std::fs::canonicalize(clean) {
        Ok(real) => temp_roots().iter().any(|root| {
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            real.starts_with(&root) && real != root
        }),
        Err(_) => true,
    }
}

fn inside_workspace(clean: &str, ws: &Path) -> bool {
    let wildcard = clean.find(['*', '?', '[']);
    let fixed = wildcard.map_or(clean, |at| clean[..at].rsplit_once('/').map_or("", |h| h.0));
    let fixed_path = Path::new(if fixed.is_empty() { "/" } else { fixed });
    let strictly = fixed_path.starts_with(ws) && (wildcard.is_some() || fixed_path != ws);
    strictly
        && !outside_workspace(
            clean.split(['*', '?', '[']).next().unwrap_or(clean),
            Some(ws),
        )
}

/// The first `rm` target in `cmd` that a recursive delete may not reach, as a model-facing
/// reason, or `None` when every target is inside the workspace or a temp directory.
pub fn out_of_bounds_delete(cmd: &str, workspace: Option<&Path>) -> Option<String> {
    let ws = workspace
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())?;
    let ws = PathBuf::from(lexical_clean(&ws.to_string_lossy()));
    let (segments, _) = effective_commands(cmd);
    for segment in &segments {
        let words = split_words(segment);
        let Some((bin, rest)) = strip_wrappers(&words).and_then(|w| w.split_first()) else {
            continue;
        };
        if bin.rsplit('/').next().unwrap_or(bin) != "rm" {
            continue;
        }
        let recursive = rest.iter().any(|w| {
            w == "--recursive"
                || (w.starts_with('-') && !w.starts_with("--") && w.contains(['r', 'R']))
        });
        if !recursive {
            continue;
        }
        for operand in rest.iter().filter(|w| !w.starts_with('-')) {
            let expanded = expand_home(operand);
            let climbs = expanded.split('/').any(|c| c == "..");
            if !(expanded.starts_with('/') || operand.starts_with('~') || climbs) {
                continue;
            }
            let absolute = if expanded.starts_with('/') {
                expanded.clone()
            } else if operand.starts_with('~') {
                // `~user/...`: another user's home is never the workspace.
                format!("/~{operand}")
            } else {
                ws.join(&expanded).to_string_lossy().into_owned()
            };
            let clean = lexical_clean(&absolute);
            let unresolved = clean.contains(['$', '`']);
            let temp_ok = !operand.starts_with('~') && !unresolved && inside_temp(&clean);
            if inside_workspace(&clean, &ws) || temp_ok {
                continue;
            }
            return Some(format!(
                "`{operand}` is outside the workspace `{}` and is not a temp directory; \
                 a recursive delete may only reach the workspace or /tmp, $TMPDIR and \
                 /var/tmp below their roots",
                ws.display()
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(cmd: &str, ws: &str) -> bool {
        out_of_bounds_delete(cmd, Some(Path::new(ws))).is_some()
    }

    #[test]
    fn workspace_and_temp_targets_run() {
        for cmd in [
            "rm -rf /app/__pycache__",
            "rm -rf /app/build /app/dist && make",
            "rm -rf /app/*",
            "rm -rf /app/a/../b",
            "rm -rf build",
            "rm -rf ./out/../build",
            "rm -fr /app/.pytest_cache",
            "rm -rf /tmp/scratch",
            "rm -rf /tmp/build-*",
            "rm -rf /var/tmp/x",
            "rm /home/u/file.txt",
            "rm -f /etc/hosts.bak",
            "ls /home",
        ] {
            assert!(!refused(cmd, "/app"), "must run: {cmd}");
        }
    }

    #[test]
    fn everything_else_is_refused_with_the_path_named() {
        for cmd in [
            "rm -rf /home/u/other-project",
            "rm -rf ~/Documents",
            "rm -rf ~/.config",
            "rm -rf /opt/app/cache",
            "rm -rf /app",
            "rm -rf /application/x",
            "rm -rf ../elsewhere",
            "rm -rf /app/../etc",
            "rm -rf /tmp",
            "rm -rf /tmp/",
            "rm -rf /tmp/*",
            "rm -rf /var/tmp",
            "rm -r /home/u/x",
            "rm --recursive /home/u/x",
            "cd /app && rm -rf /srv/data",
            "rm -rf /app/ok /home/u/not-ok",
            "rm -rf ~root/x",
        ] {
            assert!(refused(cmd, "/app"), "must be refused: {cmd}");
        }
        let reason = out_of_bounds_delete("rm -rf ~/Documents", Some(Path::new("/app"))).unwrap();
        assert!(
            reason.contains("`~/Documents`") && reason.contains("/app"),
            "{reason}"
        );
    }

    #[test]
    fn a_workspace_under_home_may_delete_inside_itself_only() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/u".into());
        let ws = format!("{}/proj", home.trim_end_matches('/'));
        assert!(!refused("rm -rf ~/proj/target", &ws));
        assert!(refused("rm -rf ~/Documents", &ws));
        assert!(refused("rm -rf ~/proj", &ws));
    }

    #[test]
    fn an_unknown_workspace_falls_back_to_the_process_directory() {
        let cwd = std::env::current_dir().unwrap();
        let inside = format!("rm -rf {}/target-x", cwd.display());
        assert!(out_of_bounds_delete(&inside, None).is_none());
        assert!(out_of_bounds_delete("rm -rf /home/u/other", None).is_some());
    }
}
