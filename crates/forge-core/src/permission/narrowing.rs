//! Where the built-in deny rules are narrower than their globs.
//!
//! The safety floor is a list of globs, and three of them refused more than they meant to:
//!
//! * `**/.env.*` and the shell verbs that read `*.env.*` also covered `.env.example`, the
//!   committed template every project keeps next to its real `.env`;
//! * `rm -rf /*` is a literal for "wipe the root", but `*` is a wildcard here, so it refused
//!   `rm -rf /app/__pycache__` and every other absolute-path delete (the structural check in
//!   `catastrophic.rs` already covers the real root, home and system directories, so the glob
//!   now fires only when that check does). The plain-text `rm -rf /` and `rm -rf ~` also fired on
//!   `rm -rf /app/x && make`, because a multi-statement line is searched for them as text;
//! * one denied path in a `read_file` batch refused the whole call, including the files that
//!   were fine to read.
//!
//! Each exception is as small as it can be: a template is only a `.env.*` file whose name ends
//! `.example`/`.sample`/`.template`/`.dist`; a scratch delete is `rm`/`rmdir` aimed only at named
//! paths under the system temp directory; a batch loses only its denied entries.

use std::borrow::Cow;
use std::path::PathBuf;

use forge_types::{is_dotenv_template, PermissionRule, RuleSource};
use serde_json::{json, Value};

use super::auto::{split_words, strip_wrappers};
use super::{denying_rule, effective_commands, lexical_clean, path_match, shell_match};

/// The built-in glob for every dotenv variant.
const DOTENV_VARIANTS: &str = "**/.env.*";
/// The built-in glob meant for `rm -rf /*`, which `*` widens to every absolute-path delete.
const ROOT_WILDCARD_DELETE: &str = "rm -rf /*";
/// The PowerShell spellings of the same literal, which `*` widens the same way.
const ROOT_WILDCARD_DELETE_PS: [&str; 2] =
    ["Remove-Item -Recurse -Force /*", "rm -Recurse -Force /*"];
/// `dd of=/dev/*` is a disk write, except for the pseudo-devices that discard or print.
const HARMLESS_DEVICES: [&str; 6] = [
    "/dev/null",
    "/dev/zero",
    "/dev/stdout",
    "/dev/stderr",
    "/dev/tty",
    "/dev/shm",
];
const DD_TO_DEVICE: &str = "dd of=/dev/*";

/// A shell word that is a dotenv template, replaced so the `*.env.*` verbs do not see it.
const TEMPLATE_WORD: &str = "<dotenv-template>";

/// Whether `pattern` (one of `rule`'s globs) matches the shell text `text` (a segment or the raw
/// line). Anything but a built-in rule matches exactly as `shell_match` does.
pub(super) fn glob_hits(rule: &PermissionRule, pattern: &str, text: &str) -> bool {
    if rule.source != RuleSource::Builtin {
        return shell_match(pattern, text);
    }
    let text = mask_public_keys(&mask_dotenv_templates(text)).into_owned();
    if !shell_match(pattern, &text) {
        return false;
    }
    if pattern == ROOT_WILDCARD_DELETE {
        return super::catastrophic::catastrophic_shell(&text).is_some();
    }
    if ROOT_WILDCARD_DELETE_PS.contains(&pattern) {
        return text.split_whitespace().any(|w| w == "/*");
    }
    if pattern == DD_TO_DEVICE {
        return !text
            .split_whitespace()
            .find_map(|w| w.strip_prefix("of="))
            .is_some_and(|target| HARMLESS_DEVICES.contains(&target));
    }
    true
}

/// The built-in literal for "wipe the root". Multi-statement lines are also searched for it as
/// plain text, which found it inside `rm -rf /tmp/scratch; make`.
const ROOT_DELETE: &str = "rm -rf /";

/// Whether the plain-text deny `pattern` occurs in the multi-statement line `cmd`.
pub(super) fn literal_hits(pattern: &str, cmd: &str) -> bool {
    if pattern == ROOT_DELETE && only_scratch_deletes(cmd) {
        return false;
    }
    if !pattern.ends_with(['/', '~']) {
        return cmd.contains(pattern);
    }
    // A pattern that names a place ends there: `rm -rf /` is the root, `rm -rf /app` is not.
    cmd.match_indices(pattern).any(|(at, _)| {
        cmd[at + pattern.len()..].chars().next().is_none_or(|c| {
            c.is_whitespace()
                || matches!(
                    c,
                    ';' | '&' | '|' | ')' | '}' | '"' | '\'' | '`' | '*' | '#'
                )
        })
    })
}

/// A word that names a public key (`id_rsa.pub`), replaced so the `*id_rsa*` verbs do not see it.
fn mask_public_keys(text: &str) -> Cow<'_, str> {
    if !text.contains(".pub") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        let bare =
            word.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '(' | ')' | ';' | '&' | '|'));
        // Only a plain `<name>.pub` path: a `$`, brace, glob or escape can splice in another word
        // (`id_rsa${IFS}.pub` is two arguments, one of them the private key).
        let plain = !bare.contains(['$', '`', '{', '}', '*', '?', '[', '\\', '=']);
        if plain && bare.ends_with(".pub") && bare.len() > ".pub".len() {
            out.push_str("<public-key>");
            out.push_str(&piece[word.len()..]);
        } else {
            out.push_str(piece);
        }
    }
    Cow::Owned(out)
}

/// Whether `pattern` matches the file path `candidate`. Anything but the built-in dotenv glob
/// matches exactly as `path_match` does.
pub(super) fn path_hits(rule: &PermissionRule, pattern: &str, candidate: &str) -> bool {
    path_match(pattern, candidate)
        && !(rule.source == RuleSource::Builtin
            && pattern == DOTENV_VARIANTS
            && is_dotenv_template(candidate))
}

/// `text` with every word that names a dotenv template replaced by [`TEMPLATE_WORD`]. A line that
/// also names a real secret (`cat .env.example .env`) still matches the rules for it.
fn mask_dotenv_templates(text: &str) -> Cow<'_, str> {
    if !text.contains(".env.") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        let bare =
            word.trim_matches(|c| matches!(c, '"' | '\'' | '`' | '(' | ')' | ';' | '&' | '|'));
        if is_dotenv_template(bare) {
            out.push_str(TEMPLATE_WORD);
            out.push_str(&piece[word.len()..]);
        } else {
            out.push_str(piece);
        }
    }
    Cow::Owned(out)
}

fn scratch_roots() -> [PathBuf; 2] {
    [PathBuf::from("/tmp"), std::env::temp_dir()]
}

/// An absolute path strictly below a scratch root, whose first component below it is a name or a
/// wildcard with a stem (`/tmp/build-*` is a named family, `/tmp/*` is everyone's files). No `..`
/// escape, no expansion that could land elsewhere, and no symlink that leaves the root.
fn is_scratch_operand(operand: &str) -> bool {
    if !operand.starts_with('/') || operand.contains(['$', '`', '~', '{']) {
        return false;
    }
    let clean = lexical_clean(operand);
    let named_below = |root: &str| {
        clean
            .strip_prefix(root.trim_end_matches('/'))
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|rest| {
                let first = rest.split('/').next().unwrap_or("");
                let literal = first
                    .chars()
                    .take_while(|c| !matches!(c, '*' | '?' | '['))
                    .count();
                // A plain name is fine; a wildcard needs a real stem before it.
                literal == first.chars().count() && literal > 0 || literal >= 3
            })
    };
    let roots = scratch_roots();
    if !roots
        .iter()
        .any(|root| named_below(&root.to_string_lossy()))
    {
        return false;
    }
    match std::fs::canonicalize(&clean) {
        Ok(real) => roots.iter().any(|root| {
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
            real.starts_with(&root) && real != root
        }),
        Err(_) => true,
    }
}

/// Whether the line deletes only scratch paths: it has at least one `rm`/`rmdir`, and every one
/// of them is aimed solely at paths [`is_scratch_operand`] accepts.
fn only_scratch_deletes(line: &str) -> bool {
    let (segments, _) = effective_commands(line);
    let mut saw_delete = false;
    for segment in &segments {
        let words = split_words(segment);
        let Some((bin, rest)) = strip_wrappers(&words).and_then(|w| w.split_first()) else {
            continue;
        };
        if !matches!(bin.rsplit('/').next().unwrap_or(bin), "rm" | "rmdir") {
            continue;
        }
        let mut operands = rest.iter().filter(|w| !w.starts_with('-')).peekable();
        if operands.peek().is_none() || !operands.all(|o| is_scratch_operand(o)) {
            return false;
        }
        saw_delete = true;
    }
    saw_delete
}

/// A batched `read_file` with the entries a deny rule refuses taken out.
pub struct DeniedPaths {
    /// The call's arguments with only the readable entries left.
    pub args: Value,
    note: String,
}

/// Split a batched `read_file` into what may be read and what a deny rule withholds. `None` when
/// nothing is denied (the call goes through as usual) or when everything is (the ordinary refusal
/// names the rule). Each entry is judged exactly as if it were a call of its own.
pub fn split_denied_paths(
    tool_name: &str,
    args: &Value,
    rules: &[PermissionRule],
) -> Option<DeniedPaths> {
    if tool_name != "read_file" || args.get("path").is_some() {
        return None;
    }
    let entries = args.get("paths")?.as_array()?;
    let mut kept = Vec::new();
    let mut withheld: Vec<String> = Vec::new();
    for entry in entries {
        let refused = entry.as_str().and_then(|path| {
            let rule = denying_rule("read_file", &json!({ "path": path }), rules)?;
            let origin = if rule.source == RuleSource::Builtin {
                "built-in"
            } else {
                "configured"
            };
            Some(format!("`{path}` ({origin} deny rule)"))
        });
        match refused {
            Some(description) => withheld.push(description),
            None => kept.push(entry.clone()),
        }
    }
    if withheld.is_empty() || kept.is_empty() {
        return None;
    }
    let note = format!(
        "[withheld by policy, {} of {} paths: {}. Only those are refused; the rest are above. \
         Do not try to reach them another way.]",
        withheld.len(),
        entries.len(),
        withheld.join(", ")
    );
    let mut narrowed = args.clone();
    narrowed["paths"] = Value::Array(kept);
    Some(DeniedPaths {
        args: narrowed,
        note,
    })
}

/// Narrow a batched `read_file` in place: a denied entry withholds only itself, and the rest of
/// the batch is read. The returned note goes on the result through [`with_withheld_note`].
pub fn narrow_read(
    tool_name: &str,
    args: &mut Value,
    rules: &[PermissionRule],
) -> Option<DeniedPaths> {
    let denied = split_denied_paths(tool_name, args, rules)?;
    args.clone_from(&denied.args);
    Some(denied)
}

/// `result` with the note appended; the direct path only gets here when the call ran.
pub fn append_withheld(result: String, withheld: Option<&DeniedPaths>) -> String {
    with_withheld_note(result, withheld, true)
}

/// `result` with the withheld-paths note appended when the narrowed call ran and succeeded.
pub fn with_withheld_note(result: String, withheld: Option<&DeniedPaths>, ok: bool) -> String {
    match withheld {
        Some(denied) if ok => format!("{result}\n\n{}", denied.note),
        _ => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::{decide, AutoVerdict};
    use forge_types::{PermissionDecision, PermissionMode, SideEffect};

    fn rules() -> Vec<PermissionRule> {
        forge_config::builtin_deny_rules()
    }

    fn read(args: Value) -> PermissionDecision {
        decide(
            PermissionMode::Bypass,
            SideEffect::ReadOnly,
            "read_file",
            &args,
            &rules(),
        )
    }

    /// A shell call in a session whose workspace is `/app`, as in the benchmark container.
    fn shell(command: &str) -> PermissionDecision {
        crate::permission::decide_in(
            PermissionMode::Bypass,
            SideEffect::Shell,
            "shell",
            &json!({ "command": command }),
            &rules(),
            Some(std::path::Path::new("/app")),
        )
    }

    #[test]
    fn env_templates_are_readable_and_real_secrets_are_not() {
        use PermissionDecision::{Allow, Deny};
        for template in [
            ".env.example",
            "./.env.example",
            "config/.env.local.sample",
            ".env.dist",
        ] {
            assert_eq!(read(json!({ "path": template })), Allow, "{template}");
        }
        for secret in [
            ".env",
            ".env.local",
            "config/.env.production",
            ".env.example.local",
            ".env.bak",
        ] {
            assert_eq!(read(json!({ "path": secret })), Deny, "{secret}");
        }
        for tool in ["list_dir", "search", "glob"] {
            let allowed = decide(
                PermissionMode::Bypass,
                SideEffect::ReadOnly,
                tool,
                &json!({ "path": ".env.example" }),
                &rules(),
            );
            assert_eq!(allowed, Allow, "{tool}");
        }
        // Writing or deleting is not reading a template: the floor keeps refusing those.
        let write = decide(
            PermissionMode::Bypass,
            SideEffect::Write,
            "write_file",
            &json!({ "path": ".env.local", "content": "x" }),
            &rules(),
        );
        assert_eq!(write, Deny);
    }

    #[test]
    fn a_line_range_does_not_hide_a_secret_from_the_rules() {
        assert_eq!(
            read(json!({ "paths": [".env:1-3"] })),
            PermissionDecision::Deny
        );
        assert_eq!(
            read(json!({ "paths": ["src/a.rs:1-3"] })),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn shell_reads_of_a_template_pass_and_reads_of_a_secret_do_not() {
        use PermissionDecision::{Allow, Deny};
        for command in [
            "cat .env.example",
            "grep -n API_URL ./config/.env.sample",
            "head -5 \"deploy/.env.template\"",
        ] {
            assert_eq!(shell(command), Allow, "{command}");
        }
        for command in [
            "cat .env",
            "cat .env.local",
            "cat .env.example .env",
            "cat .env.example; cat .env.production",
            "grep TOKEN .env.staging",
            "source .env.local",
        ] {
            assert_eq!(shell(command), Deny, "{command}");
        }
    }

    #[test]
    fn auto_mode_does_not_ask_about_a_template() {
        let verdict = |command: &str| {
            crate::permission::auto_risk(
                SideEffect::Shell,
                "shell",
                &json!({ "command": command }),
                None,
            )
        };
        assert!(
            !matches!(verdict("cat .env.example"), AutoVerdict::Risky(why) if why.contains("dotenv"))
        );
        assert!(
            matches!(verdict("cat .env.local"), AutoVerdict::Risky(why) if why.contains("dotenv"))
        );
    }

    #[test]
    fn deleting_named_scratch_paths_is_allowed() {
        use PermissionDecision::Allow;
        let tmp = std::env::temp_dir();
        let own = format!("rm -rf {}/forge-own-scratch", tmp.display());
        for command in [
            "rm -rf /tmp/sedtest; make set-log-level LEVEL=debug",
            "rm -rf /tmp/a",
            "rm -rf /tmp/forge-test-*",
            "mkdir -p /tmp/bscheck && cp a.go /tmp/bscheck/ && rm -rf /tmp/bscheck",
            "mkdir -p /tmp/bscheck\ncp a.go /tmp/bscheck/main.go\nrm -rf /tmp/bscheck",
            "rm -f /tmp/out.log /tmp/err.log",
            "rmdir /tmp/empty-dir",
            own.as_str(),
        ] {
            assert_eq!(shell(command), Allow, "{command}");
        }
    }

    #[test]
    fn every_other_absolute_delete_is_still_refused() {
        use PermissionDecision::Deny;
        for command in [
            "rm -rf /",
            "rm -rf /*",
            "rm -rf /tmp",
            "rm -rf /tmp/",
            "rm -rf /tmp/*",
            "rm -rf /tmp/?",
            "rm -rf /tmp/a*",
            "rm -rf /tmp/../etc/passwd",
            "rm -rf /tmp/x /etc/y",
            "rm -rf /tmp/x; rm -rf /usr/local/y",
            "rm -rf /tmp/x\nrm -rf /etc/y",
            "rm -rf /tmp/$NAME",
            "rm -rf /usr/local/lib/x",
            "rm -rf /etc/cron.d/y",
            "rm -rf /var",
            "rm -rf /home",
            "rm -rf /home/someone/project/build",
            "rm -rf /home/floris/Documents",
            "rm -rf /home/floris/.config",
            "rm -rf ~/Documents",
            "rm -rf ~/.config",
            "rm -rf /opt/app/cache",
            "rm -rf /var/lib/app/cache && mkdir /var/lib/app/cache",
            "rm -rf /mnt/work/target",
            "rm -rf ../elsewhere",
            "rm -rf /app/../home/u",
        ] {
            assert_eq!(shell(command), Deny, "{command}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_scratch_symlink_that_leaves_the_temp_dir_does_not_count() {
        let link = std::env::temp_dir().join(format!("forge-link-{}", forge_types::new_id()));
        std::os::unix::fs::symlink("/usr", &link).unwrap();
        let denied = shell(&format!("rm -rf {}/lib", link.display()));
        let _ = std::fs::remove_file(&link);
        assert_eq!(denied, PermissionDecision::Deny);
    }

    #[test]
    fn a_batched_read_loses_only_its_denied_entries() {
        let args = json!({ "paths": ["src/a.rs", ".env", "docs/x.md", ".env.example"] });
        let split = split_denied_paths("read_file", &args, &rules()).expect("one entry is denied");
        assert_eq!(
            split.args["paths"],
            json!(["src/a.rs", "docs/x.md", ".env.example"])
        );
        let noted = with_withheld_note("body".into(), Some(&split), true);
        assert!(noted.starts_with("body\n\n[withheld by policy, 1 of 4 paths: `.env` (built-in"));
        assert!(noted.contains("the rest are above"), "{noted}");
        assert_eq!(with_withheld_note("err".into(), Some(&split), false), "err");
    }

    #[test]
    fn a_batch_is_untouched_when_nothing_or_everything_is_denied() {
        let rules = rules();
        assert!(split_denied_paths("read_file", &json!({ "paths": ["a", "b"] }), &rules).is_none());
        assert!(
            split_denied_paths(
                "read_file",
                &json!({ "paths": [".env", ".env.local"] }),
                &rules
            )
            .is_none(),
            "an all-denied batch keeps the ordinary refusal that names the rule"
        );
        assert!(split_denied_paths("shell", &json!({ "paths": ["a", ".env"] }), &rules).is_none());
        assert!(split_denied_paths(
            "read_file",
            &json!({ "path": "a", "paths": ["a", ".env"] }),
            &rules
        )
        .is_none());
        assert!(with_withheld_note("body".into(), None, true) == "body");
    }

    #[test]
    fn a_configured_deny_rule_withholds_its_entries_too() {
        let mut rules = rules();
        rules.push(PermissionRule {
            tool: "read_file".into(),
            patterns: vec!["**/private/**".into()],
            decision: PermissionDecision::Deny,
            source: RuleSource::Configured,
            reason: None,
        });
        let split = split_denied_paths(
            "read_file",
            &json!({ "paths": ["ok.txt", "private/notes.md"] }),
            &rules,
        )
        .unwrap();
        assert_eq!(split.args["paths"], json!(["ok.txt"]));
        assert!(
            with_withheld_note(String::new(), Some(&split), true).contains("configured deny rule")
        );
    }

    #[test]
    fn real_root_and_home_wipes_are_denied_in_every_spelling() {
        for cmd in [
            "rm -rf /",
            "rm -rf /*",
            "rm -fr /",
            "rm -fr /*",
            "rm -r -f /",
            "rm -rf --no-preserve-root /",
            "rm -rf --no-preserve-root /*",
            "rm --recursive --force /",
            "rm -rf -- /",
            "/bin/rm -rf /",
            "rm -rf / --no-preserve-root",
            "rm -rf /usr",
            "rm -rf /etc/",
            "cd /app && rm -rf /",
            "rm -rf /app/cache && rm -rf /",
            "rm -rf /app/cache; rm -rf /*",
            "ls | rm -rf /",
            "rm -rf ~",
            "rm -rf ~/",
            "rm -rf ~/*",
            "rm -fr ~",
            "echo hi && rm -rf ~",
            "bash -c 'rm -rf /'",
            "echo 'x'\nrm -rf /",
            "Remove-Item -Recurse -Force /*",
            "sudo rm -rf /",
        ] {
            assert_eq!(
                shell(cmd),
                PermissionDecision::Deny,
                "must stay denied: {cmd}"
            );
        }
    }

    #[test]
    fn ordinary_absolute_path_commands_are_allowed() {
        for cmd in [
            "rm -rf /app/__pycache__",
            "rm -rf /app/__pycache__ && echo done",
            "rm -rf /app/__pycache__ /app/.pytest_cache && python -m pytest",
            "cd /app && rm -rf /app/build; make",
            "rm -rf /app/a\nrm -rf /app/b",
            "rm -rf /app/out | tee log",
            "rm -rf /var/tmp/build-1 && mkdir /var/tmp/build-1",
            "rm -rf /tmp/scratch && make",
            "rm -rf /app/a/../b && make",
            "rm -rf build dist",
            "grep 'rm -rf /' script.sh",
            "dd if=/dev/zero of=/dev/null count=1",
            "dd of=/dev/null if=/dev/urandom count=1",
            "find / -xdev -name '*.py' -size +1M 2>/dev/null | head -20",
            "find / -xdev -type f | head -50",
            "find / -xdev -maxdepth 2 -name '*.conf' | sort | head -30",
            "cat ~/.ssh/id_rsa.pub",
            "cat ~/.ssh/id_ed25519.pub ~/.ssh/id_rsa.pub",
            "grep -n Host /app/README.md",
        ] {
            assert_eq!(
                shell(cmd),
                PermissionDecision::Allow,
                "must not be denied: {cmd}"
            );
        }
    }

    #[test]
    fn a_spliced_key_name_is_not_a_public_key() {
        for cmd in [
            "cat ~/.ssh/id_rsa${IFS}.pub",
            "cat ~/.ssh/id_rsa$IFS.pub",
            "cat ~/.ssh/id_rsa$(true).pub",
            "cat ~/.ssh/id_rsa`true`.pub",
            "cat ~/.ssh/id_rsa{,.pub}",
            "cat ~/.ssh/id_rsa* .pub",
            "cat ~/.ssh/id_rsa .pub",
            "cat ~/.ssh/id_rsa.pub ~/.ssh/id_rsa",
        ] {
            assert_eq!(shell(cmd), PermissionDecision::Deny, "{cmd}");
        }
    }

    #[test]
    fn dd_to_a_real_device_stays_denied() {
        for cmd in ["dd of=/dev/sda if=disk.img", "dd of=/dev/nvme0n1 if=x"] {
            assert_eq!(shell(cmd), PermissionDecision::Deny, "{cmd}");
        }
    }

    #[test]
    fn the_refusal_names_the_exact_pattern_that_matched() {
        let rules = rules();
        let call = json!({ "command": "dd of=/dev/sda if=disk.img" });
        let rule = denying_rule("shell", &call, &rules).expect("denied");
        let text = crate::permission::rule_denial("shell", rule, &call);
        assert!(text.contains("deny rule `dd of=/dev/*`"), "{text}");
        assert!(!text.contains("mkfs"), "no unrelated patterns: {text}");
        assert!(
            text.len() < 700,
            "short enough to survive truncation: {}",
            text.len()
        );
    }

    #[test]
    fn a_configured_dotenv_glob_still_covers_templates() {
        let configured = [PermissionRule {
            tool: "read_file".into(),
            patterns: vec![DOTENV_VARIANTS.into()],
            decision: PermissionDecision::Deny,
            source: RuleSource::Configured,
            reason: None,
        }];
        let call = json!({ "path": ".env.example" });
        let decision = decide(
            PermissionMode::Bypass,
            SideEffect::ReadOnly,
            "read_file",
            &call,
            &configured,
        );
        assert_eq!(
            decision,
            PermissionDecision::Deny,
            "only the built-in rule makes the exception"
        );
    }
}
