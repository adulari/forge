//! Structural safety floor for shell lines, checked before every temper (including `bypass`).
//!
//! Two classes are refused outright, whatever the mode or the rules:
//!
//! * **catastrophic deletes** — a recursive delete, move-away or wipe of the home directory, the
//!   filesystem root or a top-level system directory. The glob denylist in `forge-config` only
//!   matched the literal `rm -rf ~`, so `rm -fr ~`, `rm -r -f ~`, `/bin/rm -rf ~`, `rm -rf $HOME`,
//!   `rm -rf ~/*` and `cd ~ && rm -rf .` all ran. This parses the flags and targets instead.
//! * **privilege escalation** — `sudo`, `su`, `doas`, `pkexec`, `run0`, `passwd`. Forge runs
//!   commands without a terminal, so these can only fail; each failed attempt is recorded by PAM as
//!   an authentication failure and three lock the account (screen unlock included). The model is
//!   told to hand the command to the user instead ([`handoff_note`]).
//!
//! Like the rest of the broker this is a floor against accidents, not a sandbox: indirection the
//! scan cannot see (`eval "$(curl …)"`, a script written to disk first) still gets through.

use std::collections::HashMap;

use super::auto::{split_words, strip_wrappers};
use super::shell_commands::{effective_commands, split_operators};

const PRIVILEGE_BINARIES: &[&str] = &["sudo", "su", "doas", "pkexec", "run0", "passwd", "sudoedit"];

const SYSTEM_DIRS: &[&str] = &[
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib64",
    "/opt",
    "/proc",
    "/root",
    "/sbin",
    "/srv",
    "/sys",
    "/usr",
    "/var",
    "/Users",
    "/System",
    "/Library",
    "/Applications",
];

/// The privilege-escalation binary a shell line would run, if any.
pub fn privilege_escalation(cmd: &str) -> Option<String> {
    let (segments, _) = effective_commands(cmd);
    segments.iter().find_map(|seg| {
        let words = split_words(seg);
        let words = strip_wrappers(&words)?;
        let bin = basename(&words[0]);
        PRIVILEGE_BINARIES.contains(&bin).then(|| bin.to_string())
    })
}

/// The message a refused privileged command returns: it tells the model to stop and hand the exact
/// command to the user, who can run it in their own terminal.
pub fn handoff_note(binary: &str, cmd: &str) -> String {
    format!(
        "refused: `{binary}` needs an interactive password prompt, and Forge runs commands without \
         a terminal, so it can only fail (and every failed attempt counts toward the account \
         lockout). Do not retry it or look for a workaround. Hand it to the user: give them the \
         exact command to run themselves in their own terminal:\n\n    {cmd}\n\nthen continue \
         once they confirm it ran."
    )
}

/// Why this shell line is a catastrophic delete, or `None`.
pub fn catastrophic_shell(cmd: &str) -> Option<String> {
    let vars = assignments(cmd);
    let (segments, _) = effective_commands(cmd);
    let all_words: Vec<Vec<String>> = segments.iter().map(|s| split_words(s)).collect();
    let mut cwd_escaped = false;
    for raw in &all_words {
        let raw = strip_grouping(raw);
        let Some(words) = strip_wrappers(&raw) else {
            continue;
        };
        let via_xargs = raw.iter().any(|w| w == "xargs");
        // `X=rm; $X -rf ~`, `$(echo rm) -rf ~`: a command word only known at run time is judged
        // as `rm` when its arguments would be catastrophic for `rm`.
        let resolved = expand(&words[0], &vars);
        let bin = if resolved.starts_with(['$', '`']) {
            "rm"
        } else {
            basename(&resolved)
        };
        let args = &words[1..];
        if bin == "cd" || bin == "pushd" {
            cwd_escaped = match args.iter().find(|a| !a.starts_with('-')) {
                None => true,
                Some(dir) => is_home_or_root(dir, &vars),
            };
            continue;
        }
        let reason = match bin {
            "rm" | "rmdir" => rm_reason(args, &vars, cwd_escaped, via_xargs, &all_words),
            "shred" => targets(args)
                .iter()
                .any(|t| is_home_or_root(t, &vars))
                .then(|| "shreds the home directory or a system directory".to_string()),
            "find" => find_reason(args, &vars),
            "mv" => {
                let paths = targets(args);
                (paths.len() >= 2
                    && paths[..paths.len() - 1]
                        .iter()
                        .any(|t| is_home_or_root(t, &vars)))
                .then(|| "moves the home directory or a system directory away".to_string())
            }
            "rsync" => {
                let paths = targets(args);
                (args.iter().any(|a| a.starts_with("--delete"))
                    && paths.last().is_some_and(|t| is_home_or_root(t, &vars)))
                .then(|| "rsync --delete into the home directory or a system directory".to_string())
            }
            "chmod" | "chown" | "chgrp" => {
                let recursive = args
                    .iter()
                    .any(|a| a == "--recursive" || (short_flag(a) && a.contains('R')));
                (recursive && targets(args).iter().any(|t| is_home_or_root(t, &vars)))
                    .then(|| "recursively changes permissions of the home directory or a system directory".to_string())
            }
            other => inline_rmtree(other, args),
        };
        if reason.is_some() {
            return reason;
        }
    }
    None
}

/// Drop subshell / group punctuation (`(rm -rf ~)`, `{ rm -rf ~; }`) so the command inside is seen.
fn strip_grouping(words: &[String]) -> Vec<String> {
    words
        .iter()
        .filter(|w| !matches!(w.as_str(), "(" | ")" | "{" | "}"))
        .map(|w| w.trim_start_matches('(').trim_end_matches(')').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

fn rm_reason(
    args: &[String],
    vars: &HashMap<String, String>,
    cwd_escaped: bool,
    via_xargs: bool,
    all_words: &[Vec<String>],
) -> Option<String> {
    let mut recursive = false;
    let mut paths = Vec::new();
    let mut end_of_options = false;
    for a in args {
        if !end_of_options && a.starts_with('#') {
            break;
        }
        if !end_of_options && a == "--" {
            end_of_options = true;
        } else if !end_of_options && a.starts_with("--") {
            recursive |= a == "--recursive";
        } else if !end_of_options && short_flag(a) {
            recursive |= a.contains(['r', 'R']);
        } else {
            paths.push(a.as_str());
        }
    }
    if !recursive {
        return None;
    }
    let joined = paths.join(" ");
    let substituted_home = (joined.contains("$(") || joined.contains('`'))
        && (joined.contains('~') || joined.contains("HOME"));
    let hit = substituted_home
        || paths.iter().any(|p| is_home_or_root(p, vars))
        || (cwd_escaped
            && paths
                .iter()
                .any(|p| matches!(*p, "." | "./" | "*" | "./*" | ".*" | "./.*")))
        || (via_xargs
            && paths.is_empty()
            && all_words
                .iter()
                .flatten()
                .any(|w| w != "xargs" && is_home_or_root(w, vars)));
    hit.then(|| {
        "recursively deletes the home directory, the filesystem root or a system directory"
            .to_string()
    })
}

fn find_reason(args: &[String], vars: &HashMap<String, String>) -> Option<String> {
    let roots: Vec<&String> = args
        .iter()
        .take_while(|a| !a.starts_with('-') && !matches!(a.as_str(), "(" | "!"))
        .collect();
    let deletes = args.iter().any(|a| a == "-delete")
        || args.windows(2).any(|w| {
            matches!(w[0].as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")
                && matches!(basename(&w[1]), "rm" | "shred" | "unlink")
        });
    (deletes && roots.iter().any(|r| is_home_or_root(r, vars)))
        .then(|| "find deletes files across the home directory or a system directory".to_string())
}

/// `python -c 'shutil.rmtree(os.path.expanduser("~"))'`, `perl -e 'rmtree($ENV{HOME})'`.
fn inline_rmtree(bin: &str, args: &[String]) -> Option<String> {
    let interpreter = bin.starts_with("python")
        || matches!(bin, "perl" | "ruby" | "node" | "nodejs" | "deno" | "bun");
    if !interpreter {
        return None;
    }
    let code = args.join(" ");
    let deletes = ["rmtree", "rm_rf", "rmSync", "rmdirSync", "remove_tree"]
        .iter()
        .any(|m| code.contains(m));
    let home_or_root = [
        "expanduser",
        "Path.home",
        "homedir",
        "Dir.home",
        "(~)",
        "(/)",
    ]
    .iter()
    .any(|m| code.contains(m))
        || (code.contains("HOME")
            && ["environ", "getenv", "ENV"]
                .iter()
                .any(|m| code.contains(m)));
    (deletes && home_or_root).then(|| {
        "inline code recursively deletes the home directory or the filesystem root".to_string()
    })
}

pub(crate) fn inline_code_deletes_home(code: &str) -> Option<String> {
    inline_rmtree("python", &[code.to_string()])
}

fn targets(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut end_of_options = false;
    for a in args {
        if !end_of_options && a == "--" {
            end_of_options = true;
        } else if end_of_options || !a.starts_with('-') {
            out.push(a.as_str());
        }
    }
    out
}

fn short_flag(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-') && !arg.starts_with("--")
}

fn basename(word: &str) -> &str {
    let word = word.trim_start_matches('\\');
    word.rsplit('/').next().unwrap_or(word)
}

/// `X=~` style assignments anywhere in the line, so `X=~; rm -rf $X` resolves `$X`.
fn assignments(cmd: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for raw in split_operators(cmd) {
        let Ok(tokens) = shell_words::split(raw.trim()) else {
            continue;
        };
        for token in tokens {
            let Some((name, value)) = token.split_once('=') else {
                break;
            };
            if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                break;
            }
            vars.insert(name.to_string(), value.to_string());
        }
    }
    vars
}

/// Replace `$HOME`/`${HOME…}` with `~` and known `$X`/`${X}` with their assigned value.
fn expand(target: &str, vars: &HashMap<String, String>) -> String {
    let mut out = target.to_string();
    for _ in 0..3 {
        let before = out.clone();
        if let Some(rest) = out.strip_prefix("${HOME") {
            if let Some(end) = rest.find('}') {
                out = format!("~{}", &rest[end + 1..]);
            }
        }
        if let Some(rest) = out.strip_prefix("$HOME") {
            if !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                out = format!("~{rest}");
            }
        }
        for (name, value) in vars {
            for form in [format!("${{{name}}}"), format!("${name}")] {
                if let Some(rest) = out.strip_prefix(form.as_str()) {
                    if form.ends_with('}')
                        || !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')
                    {
                        out = format!("{value}{rest}");
                    }
                }
            }
        }
        if out == before {
            break;
        }
    }
    out
}

fn is_home_or_root(target: &str, vars: &HashMap<String, String>) -> bool {
    if (target.contains("$(") || target.contains('`'))
        && (target.contains('~') || target.contains("HOME"))
    {
        return true;
    }
    let mut t = expand(target.trim(), vars);
    if t.starts_with('/') {
        t = super::lexical_clean(&t);
    }
    loop {
        let before = t.len();
        for suffix in ["/*", "/.*", "/.", "/"] {
            if t.len() > suffix.len() && t.ends_with(suffix) {
                t.truncate(t.len() - suffix.len());
            }
        }
        if t.len() == before {
            break;
        }
    }
    if matches!(t.as_str(), "" | "/" | "/*" | "/.*" | "~" | "~/" | "~/*") {
        return !target.trim().is_empty();
    }
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok()) {
        let home = home.trim_end_matches('/');
        if !home.is_empty() && t == home {
            return true;
        }
    }
    SYSTEM_DIRS.contains(&t.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_root_delete_spellings_are_caught() {
        for cmd in [
            "rm -rf ~",
            "rm -fr ~",
            "rm -r -f ~",
            "rm -Rf ~",
            "rm --recursive --force ~",
            "rm -rf ~/",
            "rm -rf ~/*",
            "rm -rf -- ~",
            "rm -rf $HOME",
            "rm -rf \"$HOME\"",
            "rm -rf ${HOME}",
            "rm -rf \"${HOME:?}\"/",
            "rm -rf $(echo ~)",
            "/bin/rm -rf ~",
            "\\rm -rf ~",
            "command rm -rf ~",
            "env rm -rf ~",
            "timeout 5 rm -rf ~",
            "nice rm -rf ~",
            "cd ~ && rm -rf .",
            "cd; rm -rf *",
            "cd ~ && rm -rf ./*",
            "cd $HOME; rm -rf .",
            "find ~ -delete",
            "find ~ -mindepth 1 -exec rm -rf {} +",
            "echo ~ | xargs rm -rf",
            "X=~; rm -rf $X",
            "bash -c 'rm -rf ~'",
            "rm -rf /",
            "rm -rf /*",
            "rm -rf /usr",
            "rm -rf /etc/",
            "mv ~ ~.moved",
            "rsync -a --delete /var/empty/ ~/",
            "chmod -R 000 ~",
            "shred -u ~/*",
            "python3 -c 'import shutil,os;shutil.rmtree(os.path.expanduser(\"~\"))'",
            "perl -MFile::Path -e 'rmtree($ENV{HOME})'",
            "rm -rf ~ # cleanup",
            "true && rm -rf ~",
            "(rm -rf ~)",
            "{ rm -rf ~; }",
            "X=rm; $X -rf ~",
            "$(echo rm) -rf ~",
            "`echo rm` -rf ~",
            "if true; then rm -rf ~; fi",
            "for d in x; do rm -rf /etc; done",
            "while true; do nice -n 5 rm -rf ~; done",
            "true && nice -n 5 rm -rf /usr",
            "! rm -rf ~",
            "env -u FOO rm -rf /",
            "ionice -c 3 rm -rf ~/",
        ] {
            assert!(catastrophic_shell(cmd).is_some(), "not caught: {cmd}");
        }
    }

    #[test]
    fn ordinary_deletes_are_not_catastrophic() {
        for cmd in [
            "rm -rf target",
            "rm -rf ./node_modules",
            "rm -rf ~/projects/old-build",
            "rm -rf $HOME/.cache/forge-test",
            "rm -rf /tmp/scratch",
            "rm ~/notes.txt",
            "cd src && rm -rf generated",
            "rm -rf .",
            "find . -name '*.o' -delete",
            "find ~/proj -name x",
            "mv ~/a.txt ~/b.txt",
            "rsync -a --delete build/ out/",
            "chmod -R u+w target",
            "grep 'rm -rf ~' notes.md",
            "echo rm -rf ~",
            "python3 -c 'import shutil; shutil.rmtree(\"build\")'",
        ] {
            assert_eq!(catastrophic_shell(cmd), None, "false positive: {cmd}");
        }
    }

    #[test]
    fn privilege_escalation_is_found_through_wrappers() {
        for (cmd, bin) in [
            ("sudo -n true", "sudo"),
            ("sudo apt install x", "sudo"),
            ("cargo build && sudo make install", "sudo"),
            ("echo pw | sudo -S true", "sudo"),
            ("bash -c 'sudo rm x'", "sudo"),
            ("env FOO=1 sudo true", "sudo"),
            ("/usr/bin/sudo true", "sudo"),
            ("su -c id", "su"),
            ("pkexec true", "pkexec"),
            ("doas true", "doas"),
            ("run0 true", "run0"),
            ("passwd", "passwd"),
            ("timeout 5 sudo true", "sudo"),
            ("echo x | xargs sudo rm", "sudo"),
        ] {
            assert_eq!(privilege_escalation(cmd).as_deref(), Some(bin), "{cmd}");
        }
        for cmd in [
            "man sudo",
            "which sudo",
            "grep sudo /etc/group",
            "echo sudo",
            "git commit -m 'sudo'",
        ] {
            assert_eq!(privilege_escalation(cmd), None, "{cmd}");
        }
    }
}
