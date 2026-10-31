//! Where a recursive delete may reach: the session workspace and the temp roots, nowhere else.
//!
//! `catastrophic.rs` refuses the home directory itself, the root and system trees. That leaves
//! `rm -rf ~/Documents` and `rm -rf /home/u/other-project` runnable, so a recursive delete is
//! also refused (in every temper, `bypass` included) when an absolute or `~` target is not
//! strictly inside the workspace or a temp directory, or when a relative target climbs out of the
//! workspace with `..`. An unknown workspace falls back to the process directory, never to
//! "anything goes".

use std::path::{Path, PathBuf};

use super::auto::{split_words, strip_wrappers};
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

/// `path` with its longest existing ancestor resolved through symlinks and the rest appended
/// lexically, so a spelling through a symlink and a target that does not exist yet both compare
/// by where they really are.
fn real_path(path: &str) -> PathBuf {
    let mut rest: Vec<String> = Vec::new();
    let mut head = PathBuf::from(path);
    loop {
        if let Ok(real) = std::fs::canonicalize(&head) {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (head.file_name(), head.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_string_lossy().into_owned());
                head = parent.to_path_buf();
            }
            _ => return PathBuf::from(path),
        }
    }
}

/// A workspace that is `/`, the home directory or an ancestor of it grants nothing: every
/// absolute delete would be "inside" it, which is the unbounded case this check exists to stop.
fn workspace_is_too_broad(ws: &Path) -> bool {
    if ws == Path::new("/") {
        return true;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return false;
    }
    let home = real_path(&home);
    let real = real_path(&ws.to_string_lossy());
    home.starts_with(&real)
}

fn inside_workspace(clean: &str, ws: &Path) -> bool {
    if workspace_is_too_broad(ws) {
        return false;
    }
    let wildcard = clean.find(['*', '?', '[']);
    let fixed = wildcard.map_or(clean, |at| clean[..at].rsplit_once('/').map_or("", |h| h.0));
    let real_fixed = real_path(if fixed.is_empty() { "/" } else { fixed });
    let real_ws = real_path(&ws.to_string_lossy());
    real_fixed.starts_with(&real_ws) && (wildcard.is_some() || real_fixed != real_ws)
}

fn join_cwd(base: &Path, dir: &str) -> PathBuf {
    let expanded = expand_home(dir);
    let joined = if expanded.starts_with('/') {
        PathBuf::from(expanded)
    } else {
        base.join(expanded)
    };
    PathBuf::from(lexical_clean(&joined.to_string_lossy()))
}

/// The first `rm` target in `cmd` that a recursive delete may not reach, as a model-facing
/// reason, or `None` when every target is inside the workspace or a temp directory. `cwd` is the
/// shell call's own working-directory argument; `cd` statements inside `cmd` move it further.
pub fn out_of_bounds_delete(
    cmd: &str,
    workspace: Option<&Path>,
    cwd: Option<&str>,
) -> Option<String> {
    let ws = workspace
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())?;
    let ws = PathBuf::from(lexical_clean(&ws.to_string_lossy()));
    let mut here = cwd.map_or_else(|| ws.clone(), |dir| join_cwd(&ws, dir));
    let (segments, _) = effective_commands(cmd);
    for segment in &segments {
        let words = split_words(segment);
        let command = strip_wrappers(&words).unwrap_or(&words);
        if matches!(command.first().map(String::as_str), Some("cd" | "pushd")) {
            if let Some(dir) = command[1..].iter().find(|w| !w.starts_with('-')) {
                here = join_cwd(&here, dir);
            }
            continue;
        }
        let is_rm = command
            .first()
            .is_some_and(|bin| bin.rsplit('/').next().unwrap_or(bin) == "rm");
        if let Some(rest) = is_rm.then(|| &command[1..]) {
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
                    here.join(&expanded).to_string_lossy().into_owned()
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
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(cmd: &str, ws: &str) -> bool {
        out_of_bounds_delete(cmd, Some(Path::new(ws)), None).is_some()
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
        let reason =
            out_of_bounds_delete("rm -rf ~/Documents", Some(Path::new("/app")), None).unwrap();
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
        assert!(out_of_bounds_delete(&inside, None, None).is_none());
        assert!(out_of_bounds_delete("rm -rf /home/u/other", None, None).is_some());
    }

    #[test]
    fn control_words_and_wrapper_options_do_not_hide_a_delete() {
        let prefixes = [
            "",
            "true && ",
            "if [ -d /etc/foo ]; then ",
            "for d in x; do ",
            "while true; do ",
            "{ ",
            "( ",
            "! ",
            "nice -n 5 ",
            "nice -5 ",
            "ionice -c 3 ",
            "env -u FOO ",
            "env -i A=b ",
            "nohup ",
            "time ",
            "command ",
            "exec ",
            "xargs -0 ",
            "timeout 5 ",
            "setsid ",
            "stdbuf -o L ",
            "FOO=1 ",
            "true && nice -n 5 ",
            "if true; then nice -n 5 ",
        ];
        let suffixes = ["", "; fi", "; done", "; }", " )"];
        for prefix in prefixes {
            for suffix in suffixes {
                for target in ["/etc", "/home/u/Documents", "~/Documents", "/opt/app/cache"] {
                    let cmd = format!("{prefix}rm -rf {target}{suffix}");
                    assert!(refused(&cmd, "/app"), "must be refused: {cmd}");
                }
                let inside = format!("{prefix}rm -rf /app/build{suffix}");
                assert!(!refused(&inside, "/app"), "must run: {inside}");
            }
        }
    }

    #[test]
    fn a_workspace_that_is_home_or_root_grants_nothing() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/u".into());
        let home = home.trim_end_matches('/');
        for ws in [home, "/"] {
            for cmd in [
                format!("rm -rf {home}/Documents"),
                "rm -rf ~/.config".to_string(),
                "rm -rf /usr/lib/x".to_string(),
                "rm -rf /opt/app".to_string(),
            ] {
                assert!(refused(&cmd, ws), "ws={ws}: {cmd}");
            }
            assert!(!refused("rm -rf /tmp/scratch", ws), "temp stays open");
            assert!(!refused("rm -rf build", ws), "relative stays open");
        }
    }

    #[test]
    fn a_cwd_argument_or_cd_moves_where_dotdot_lands() {
        let ws = Path::new("/app");
        let run = |cmd: &str, cwd: Option<&str>| out_of_bounds_delete(cmd, Some(ws), cwd);
        assert!(run("rm -rf ../dist", Some("frontend")).is_none());
        assert!(run("cd frontend && rm -rf ../dist", None).is_none());
        assert!(run("cd frontend; cd src; rm -rf ../../dist", None).is_none());
        assert!(run("rm -rf ../dist", None).is_some());
        assert!(run("rm -rf ../../etc", Some("frontend")).is_some());
        assert!(run("cd /etc && rm -rf ../app/x", None).is_none());
        assert!(run("cd /etc && rm -rf ../usr", None).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_spellings_compare_by_where_they_really_are() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("target-wsdel-{}", forge_types::new_id()));
        let real = base.join("data/proj");
        std::fs::create_dir_all(real.join("target")).unwrap();
        std::fs::create_dir_all(base.join("outside/keep")).unwrap();
        std::os::unix::fs::symlink(base.join("data"), base.join("alias")).unwrap();
        std::os::unix::fs::symlink(base.join("outside"), real.join("escape")).unwrap();
        let ws = real.canonicalize().unwrap();
        let via_alias = format!("rm -rf {}/alias/proj/target", base.display());
        let via_escape = format!("rm -rf {}/escape/keep", real.display());
        let in_ws = out_of_bounds_delete(&via_alias, Some(&ws), None);
        let out = out_of_bounds_delete(&via_escape, Some(&ws), None);
        let _ = std::fs::remove_dir_all(&base);
        assert!(in_ws.is_none(), "{in_ws:?}");
        assert!(out.is_some(), "a link that leaves the workspace is refused");
    }
}
