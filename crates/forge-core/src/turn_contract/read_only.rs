//! Whether a prompt explicitly asks for a turn that changes nothing.
//!
//! A read-only contract removes write capability for the whole turn: [`crate::TaskScope`] denies
//! every mutating tool and the model is told to change nothing. Getting it wrong in the permissive
//! direction costs a missing guarantee; getting it wrong in the restrictive direction costs the
//! whole task, because a change request that ends "do not modify anything under `logs/`, `ops/` or
//! `data/`, nor README.md, ..." is then answered with a diagnosis and no fix. So the classifier
//! reads a prohibition as read-only only when it names no part of the tree: a named path, a list of
//! protected files, a narrowing tail ("under", "outside", "other than", "else") or an object it does
//! not recognise all make it a constraint on a change request instead. Doubt resolves to "not
//! read-only".

mod prohibition;

use prohibition::forbids_writing_everywhere;

/// Phrases that put the whole turn under a read-only contract by construction, so they are safe to
/// match anywhere in the prompt. A protected thing right after one ("this is read-only data")
/// makes it a description of that thing, which is not an instruction about the turn.
const READ_ONLY_DIRECTIVES: [&str; 13] = [
    "this is read-only",
    "this is read only",
    "this is a read-only",
    "this is a read only",
    "this task is read-only",
    "this request is read-only",
    "this turn is read-only",
    "treat this as read-only",
    "treat this as read only",
    "keep this read-only",
    "keep this read only",
    "keep it read-only",
    "keep it read only",
];

/// Bare read-only tokens. These are ordinary technical vocabulary — a bug report says the sandbox
/// stays read-only, or that a turn silently ran READ-ONLY — so they only count as a contract when
/// they OPEN a segment, i.e. when they address this turn rather than describe something.
const READ_ONLY_ANCHORS: [&str; 2] = ["read-only", "read only"];

/// What "read-only" qualifies when it is an adjective on a thing rather than an instruction:
/// "Read-only files: `data/`, `logs/`" lists protected paths, "read-only access to the DB" says what
/// the task may use.
const PROTECTED_THINGS: [&str; 52] = [
    "files",
    "file",
    "directories",
    "directory",
    "dirs",
    "dir",
    "folders",
    "folder",
    "paths",
    "path",
    "mounts",
    "mount",
    "volumes",
    "volume",
    "fixtures",
    "fixture",
    "tests",
    "test",
    "data",
    "inputs",
    "input",
    "assets",
    "resources",
    "copies",
    "copy",
    "config",
    "configs",
    "configuration",
    "settings",
    "modules",
    "packages",
    "repos",
    "repo",
    "repositories",
    "checkout",
    "clone",
    "vendor",
    "vendored",
    "access",
    "database",
    "databases",
    "db",
    "tables",
    "views",
    "columns",
    "fields",
    "replica",
    "snapshots",
    "secrets",
    "credentials",
    "filesystem",
    "container",
];

/// Whether the prompt tells the model to change nothing. See the module docs for what counts.
pub(super) fn explicitly_read_only(prompt: &str) -> bool {
    let lower = normalize(prompt);
    names_a_read_only_turn(&lower) || forbids_writing_everywhere(&lower)
}

fn normalize(prompt: &str) -> String {
    let mut out = String::with_capacity(prompt.len());
    for c in prompt.chars() {
        let c = match c {
            '\u{2019}' | '\u{2018}' => '\'',
            '\t' => ' ',
            other => other.to_ascii_lowercase(),
        };
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out
}

/// `text` after `word` when `word` opens it as a whole word.
pub(super) fn word_prefix<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    text.strip_prefix(word).filter(|rest| {
        rest.chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && !matches!(c, '\'' | '_' | '-' | '/'))
    })
}

pub(super) fn before_is_word_edge(text: &str, at: usize) -> bool {
    text[..at]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric() && !matches!(c, '\'' | '_' | '-'))
}

pub(super) fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric())
}

/// The first word after a read-only token qualifies a thing ("read-only files"), so the token
/// describes that thing instead of addressing the turn.
fn names_a_protected_thing(after_token: &str) -> bool {
    if !after_token.starts_with(' ') {
        return false;
    }
    after_token
        .split_whitespace()
        .next()
        .is_some_and(|word| PROTECTED_THINGS.contains(&bare(word)))
}

fn names_a_read_only_turn(lower: &str) -> bool {
    let stated = READ_ONLY_DIRECTIVES.iter().any(|phrase| {
        lower
            .match_indices(phrase)
            .any(|(at, _)| !names_a_protected_thing(&lower[at + phrase.len()..]))
    });
    stated
        || lower.split(['\n', '.', '!', '?', ';']).any(|segment| {
            let segment = trim_directive_decoration(segment);
            READ_ONLY_ANCHORS.iter().any(|anchor| {
                segment
                    .strip_prefix(anchor)
                    .is_some_and(|rest| !names_a_protected_thing(rest))
            })
        })
}

/// Strip list bullets, markdown emphasis and one leading framing label so an anchored directive is
/// still recognized as `- **Read-only**: ...` or `Note: read only`.
pub(super) fn trim_directive_decoration(segment: &str) -> &str {
    let segment = segment
        .trim()
        .trim_start_matches(|c: char| {
            matches!(c, '-' | '*' | '#' | '>' | '`' | '_' | '[' | '(')
                || c.is_ascii_digit()
                || c.is_whitespace()
        })
        .trim_start();
    for label in [
        "note:",
        "important:",
        "constraint:",
        "scope:",
        "mode:",
        "please",
    ] {
        if let Some(rest) = segment.strip_prefix(label) {
            return rest.trim_start();
        }
    }
    segment
}

#[cfg(test)]
#[path = "read_only_tests.rs"]
mod tests;
