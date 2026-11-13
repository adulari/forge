//! Auto-edit's narrower shell check (`accept-edits`): only deletes, moves and writes that leave
//! the workspace, raw disk writes, pipes into a shell and inline deleting code ask.

use super::*;

/// The narrower check behind Auto-edit's shell (`accept-edits`): ordinary commands run without a
/// prompt, but a line that deletes, moves or writes outside the workspace, touches a raw disk,
/// pipes into a shell, or runs inline code that deletes files asks first. Privilege escalation and
/// catastrophic deletes never reach here (refused by the floor in `catastrophic.rs`).
pub fn accept_edits_risk(cmd: &str, ws: Option<&Path>) -> Option<String> {
    if pipes_into_shell(cmd) {
        return Some("pipes content into a shell".into());
    }
    if let Some(target) = redirect_targets(cmd)
        .into_iter()
        .find(|t| outside_workspace(t, ws))
    {
        return Some(format!("redirects output outside the workspace: {target}"));
    }
    let (segments, _) = effective_commands(cmd);
    let mut cwd_outside = false;
    for seg in &segments {
        let words = split_words(seg);
        let Some(words) = strip_wrappers(&words) else {
            continue;
        };
        let Some((cmd_word, rest)) = words.split_first() else {
            continue;
        };
        let via_xargs = split_words(seg).iter().any(|w| w == "xargs");
        let cmd = cmd_word.rsplit('/').next().unwrap_or(cmd_word);
        let flags: Vec<&str> = rest
            .iter()
            .filter(|w| w.starts_with('-'))
            .map(String::as_str)
            .collect();
        let paths: Vec<&str> = rest
            .iter()
            .filter(|w| !w.starts_with('-') && !w.contains('='))
            .map(String::as_str)
            .collect();
        if matches!(cmd, "cd" | "pushd") {
            cwd_outside = paths.first().is_none_or(|d| outside_workspace(d, ws));
            continue;
        }
        let outside =
            |p: &str| outside_workspace(p, ws) || (cwd_outside && !Path::new(p).is_absolute());
        let has_short = |c: char| {
            flags
                .iter()
                .any(|f| !f.starts_with("--") && f[1..].contains(c))
        };
        let reason: Option<String> = match cmd {
            "rm" | "rmdir" | "unlink" | "shred" => {
                let forced = cmd == "shred"
                    || has_short('r')
                    || has_short('R')
                    || has_short('f')
                    || flags
                        .iter()
                        .any(|f| matches!(*f, "--recursive" | "--force" | "--no-preserve-root"));
                let blind = via_xargs && paths.is_empty();
                (forced && (blind || paths.iter().any(|p| outside(p))))
                    .then(|| "recursive or forced delete outside the workspace".to_string())
            }
            "find" => {
                let roots: Vec<&String> = rest
                    .iter()
                    .take_while(|w| !w.starts_with('-') && !matches!(w.as_str(), "(" | "!"))
                    .collect();
                let deletes = rest.iter().any(|w| w == "-delete")
                    || rest.windows(2).any(|w| {
                        matches!(w[0].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")
                            && matches!(
                                w[1].rsplit('/').next().unwrap_or(&w[1]),
                                "rm" | "shred" | "unlink" | "mv"
                            )
                    });
                (deletes && ((roots.is_empty() && cwd_outside) || roots.iter().any(|r| outside(r))))
                    .then(|| "find deletes files outside the workspace".to_string())
            }
            "dd" if rest.iter().any(|w| w.starts_with("of=")) => {
                Some("raw disk write (dd of=)".into())
            }
            "mkfs" | "wipefs" | "fdisk" | "parted" | "sfdisk" | "gdisk" => {
                Some("disk-level operation".into())
            }
            c if c.starts_with("mkfs.") => Some("disk-level operation".into()),
            c if inline_deletes(c, rest) => {
                Some("runs inline interpreter code that deletes files".into())
            }
            "rsync" if flags.iter().any(|f| f.starts_with("--delete")) => paths
                .last()
                .filter(|p| outside(p))
                .map(|p| format!("rsync --delete outside the workspace: {p}")),
            _ => None,
        };
        if reason.is_some() {
            return reason;
        }
        let dests: Vec<&str> = match cmd {
            "cp" | "install" | "ln" | "rsync" => paths.last().copied().into_iter().collect(),
            "mv" | "touch" | "mkdir" | "tee" | "chmod" | "chown" | "chgrp" | "truncate" => {
                paths.clone()
            }
            "sed"
                if flags
                    .iter()
                    .any(|f| f.starts_with("-i") || *f == "--in-place") =>
            {
                paths.last().copied().into_iter().collect()
            }
            _ => Vec::new(),
        };
        if let Some(p) = dests.into_iter().find(|p| outside(p)) {
            return Some(format!("`{cmd}` writes outside the workspace: {p}"));
        }
    }
    None
}

/// `python -c`/`node -e`/`perl -e` code that deletes files (a narrower cut of
/// [`inline_code_risk`], which also flags spawning and the network).
fn inline_deletes(cmd: &str, rest: &[String]) -> bool {
    let interpreter = cmd.starts_with("python")
        || matches!(
            cmd,
            "node" | "nodejs" | "deno" | "bun" | "perl" | "ruby" | "php"
        );
    if !interpreter {
        return false;
    }
    let Some(pos) = rest
        .iter()
        .position(|w| matches!(w.as_str(), "-c" | "-e" | "-r" | "-E" | "--eval"))
    else {
        return false;
    };
    let code = rest[pos + 1..].join(" ").to_ascii_lowercase();
    [
        "rmtree",
        "unlink",
        "rmdir",
        "remove(",
        "rm -",
        "rm_rf",
        "rmsync",
        ".rm(",
        "fs.rm",
        "deno.remove",
    ]
    .iter()
    .any(|m| code.contains(m))
}
