//! Write prohibitions: "do not edit ...", "without changing ...", "make no edits ...", and whether
//! each one forbids writing everywhere or only somewhere.

use super::{bare, before_is_word_edge, trim_directive_decoration, word_prefix};

/// Words that open a prohibition addressed to the model.
const NEGATORS: [&str; 3] = ["do not", "don't", "dont"];

/// Verbs whose prohibition forbids writing. `write` is handled apart: "do not write anything" is
/// often about output, only "write files" is about the tree.
const WRITE_VERBS: [&str; 5] = ["edit", "modify", "change", "touch", "alter"];

/// Verbs a prohibition lists after its first one: "do not edit, delete, or rename the existing tests".
const COORDINATED_VERBS: [&str; 25] = [
    "delete",
    "remove",
    "rename",
    "modify",
    "change",
    "touch",
    "overwrite",
    "rewrite",
    "refactor",
    "move",
    "alter",
    "edit",
    "reformat",
    "relocate",
    "replace",
    "update",
    "add",
    "create",
    "write",
    "drop",
    "patch",
    "tweak",
    "adjust",
    "reorder",
    "reset",
];

/// Objects that name a kind of content or the project as a whole ("it" joins them: "don't edit it"
/// points at whatever the prompt named last). "Do not touch the code" ends a request to update the
/// docs as often as it ends a request to explain the code, so these only mean "everything" when the
/// prompt asks for no change or the sentence closes the ban itself.
const WEAK_OBJECTS: [&str; 11] = [
    "source code",
    "working directory",
    "working tree",
    "code base",
    "codebase",
    "repository",
    "workspace",
    "project",
    "source",
    "code",
    "repo",
];

/// Words that make a universal object part of the tree: "in tests/" and "except ANSWER.md" are
/// handled by never matching them as fillers; this list covers the ones that follow a comma or a
/// parenthesis ("do not modify anything, except `CHANGELOG.md`").
const NARROWING_AFTER_BOUNDARY: [&str; 9] = [
    "except",
    "excepting",
    "excluding",
    "apart",
    "aside",
    "besides",
    "other",
    "save",
    "outside",
];

/// Phrases that follow a universal object and restate the ban instead of narrowing it: "in any
/// way", "in place", "yet", "in the repo".
const FILLERS: [&str; 50] = [
    "at all",
    "whatsoever",
    "in any way",
    "in any form",
    "in any manner",
    "in any fashion",
    "in any shape or form",
    "in any case",
    "in place",
    "for now",
    "right now",
    "now",
    "yet",
    "for the time being",
    "at this point",
    "at any point",
    "at any time",
    "at any stage",
    "under any circumstances",
    "anywhere in the repo",
    "anywhere in the repository",
    "anywhere in the project",
    "anywhere",
    "in any of the files",
    "in any file",
    "in the repo",
    "in this repo",
    "in the repository",
    "in this repository",
    "in the project",
    "in this project",
    "in the codebase",
    "in this codebase",
    "in the workspace",
    "in this workspace",
    "in the working tree",
    "in the working directory",
    "in the current directory",
    "in this directory",
    "on disk",
    "please",
    "thanks",
    "though",
    "this time",
    "this turn",
    "in this turn",
    "for this turn",
    "for this task",
    "without asking",
    "without permission",
];

/// Fillers that can stand in for the object itself: "make no edits whatsoever", "do not edit yet".
const BARE_FILLERS: [&str; 10] = [
    "at all",
    "whatsoever",
    "yet",
    "for now",
    "right now",
    "now",
    "for the time being",
    "at this point",
    "under any circumstances",
    "please",
];

/// What a ban may be followed by when the next clause turns to answering.
const ACTIONS: [&str; 34] = [
    "explain",
    "tell",
    "report",
    "summarize",
    "summarise",
    "describe",
    "list",
    "show",
    "answer",
    "give",
    "say",
    "print",
    "stop",
    "read",
    "look",
    "review",
    "audit",
    "inspect",
    "analyze",
    "analyse",
    "walk",
    "outline",
    "check",
    "find",
    "investigate",
    "diagnose",
    "run",
    "reply",
    "respond",
    "trace",
    "compare",
    "identify",
    "examine",
    "explore",
];

/// Imperatives that ask for a change when they open a sentence.
const CHANGE_VERBS: [&str; 30] = [
    "fix",
    "implement",
    "add",
    "update",
    "refactor",
    "rename",
    "remove",
    "create",
    "write",
    "change",
    "modify",
    "replace",
    "edit",
    "delete",
    "rewrite",
    "extend",
    "patch",
    "insert",
    "debug",
    "repair",
    "build",
    "optimize",
    "optimise",
    "migrate",
    "port",
    "upgrade",
    "convert",
    "resolve",
    "correct",
    "make",
];

/// Phrases that lead into a request without being part of it.
const LEAD_INS: [&str; 18] = [
    "can you ",
    "could you ",
    "would you ",
    "will you ",
    "i need you to ",
    "i want you to ",
    "you should ",
    "you need to ",
    "go ahead and ",
    "also ",
    "then ",
    "now ",
    "but ",
    "so ",
    "please ",
    "first ",
    "next ",
    "finally ",
];

/// Whether some sentence of the prompt opens with an imperative that asks for a change.
fn requests_change(lower: &str) -> bool {
    let text = lower
        .replace(" and ", ". ")
        .replace(", then ", ". ")
        .replace(" then ", ". ");
    text.split(['\n', '.', '!', '?', ';', ':']).any(|segment| {
        let mut segment = trim_directive_decoration(segment);
        while let Some(rest) = LEAD_INS.iter().find_map(|lead| segment.strip_prefix(lead)) {
            segment = rest.trim_start();
        }
        let Some(first) = segment.split_whitespace().next() else {
            return false;
        };
        CHANGE_VERBS.contains(&bare(first)) && !segment.starts_with("make no ")
    })
}

/// A write prohibition's scope: the text after its verb, whether "to" may introduce the object
/// ("make changes to anything") and whether only a file object forbids everything.
struct Prohibition<'a> {
    rest: &'a str,
    takes_to: bool,
    files_only: bool,
}

/// Parses what follows a negator ("do not", "don't") as a write prohibition.
fn after_negator(rest: &str) -> Option<Prohibition<'_>> {
    let mut rest = rest.trim_start();
    while let Some(after) = ["ever", "please"]
        .iter()
        .find_map(|adverb| word_prefix(rest, adverb))
    {
        rest = after.trim_start();
    }
    if let Some(after) = WRITE_VERBS.iter().find_map(|verb| word_prefix(rest, verb)) {
        return Some(Prohibition {
            rest: after,
            takes_to: false,
            files_only: false,
        });
    }
    if let Some(after) = word_prefix(rest, "write") {
        return Some(Prohibition {
            rest: after,
            takes_to: true,
            files_only: true,
        });
    }
    word_prefix(rest, "make").and_then(|after| after_make(after.trim_start(), false))
}

/// "make [any] changes|edits|modifications" and its "no" and gerund forms; `after` follows "make".
fn after_make(after: &str, expect_no: bool) -> Option<Prohibition<'_>> {
    let mut after = after;
    if expect_no {
        after = word_prefix(after, "no")?.trim_start();
    } else {
        for filler in ["any", "further", "more", "additional"] {
            if let Some(rest) = word_prefix(after, filler) {
                after = rest.trim_start();
            }
        }
    }
    [
        "changes",
        "edits",
        "modifications",
        "code changes",
        "file changes",
        "edit",
        "change",
    ]
    .iter()
    .find_map(|noun| word_prefix(after, noun))
    .map(|rest| Prohibition {
        rest,
        takes_to: true,
        files_only: false,
    })
}

/// Parses what follows "without" as a write prohibition: "without changing files".
fn after_without(rest: &str) -> Option<Prohibition<'_>> {
    let rest = rest.trim_start();
    if let Some(after) = ["changing", "modifying", "editing", "touching", "altering"]
        .iter()
        .find_map(|gerund| word_prefix(rest, gerund))
    {
        return Some(Prohibition {
            rest: after,
            takes_to: false,
            files_only: false,
        });
    }
    if let Some(after) = word_prefix(rest, "writing") {
        return Some(Prohibition {
            rest: after,
            takes_to: true,
            files_only: true,
        });
    }
    word_prefix(rest, "making").and_then(|after| {
        let after = after.trim_start();
        after_make_noun(word_prefix(after, "any").map_or(after, str::trim_start))
    })
}

fn after_make_noun(text: &str) -> Option<Prohibition<'_>> {
    ["changes", "edits", "modifications"]
        .iter()
        .find_map(|noun| word_prefix(text, noun))
        .map(|rest| Prohibition {
            rest,
            takes_to: true,
            files_only: false,
        })
}

pub(super) fn forbids_writing_everywhere(lower: &str) -> bool {
    let mut cache = None;
    let mut change_requested = || *cache.get_or_insert_with(|| requests_change(lower));
    let mut found = Vec::new();
    for negator in NEGATORS {
        for (at, _) in lower.match_indices(negator) {
            let end = at + negator.len();
            if before_is_word_edge(lower, at)
                && lower[end..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric())
            {
                found.extend(after_negator(&lower[end..]));
            }
        }
    }
    for (at, _) in lower.match_indices("without ") {
        if before_is_word_edge(lower, at) {
            found.extend(after_without(&lower[at + "without ".len()..]));
        }
    }
    for (at, _) in lower.match_indices("make no ") {
        if before_is_word_edge(lower, at) {
            found.extend(after_make(&lower[at + "make ".len()..], true));
        }
    }
    found
        .into_iter()
        .any(|prohibition| prohibition_is_universal(&prohibition, &mut change_requested))
}

/// The verbs coordinated with the first one, removed, so the comma after "edit" in "do not edit,
/// delete, or rename the existing tests" no longer reads as the end of the clause. Text without such
/// a verb list is returned as it came.
fn after_coordinated_verbs(rest: &str) -> &str {
    let mut cursor = rest;
    let mut skipped_any = false;
    loop {
        let trimmed = cursor.trim_start();
        let (had_comma, next) = match trimmed.strip_prefix(',') {
            Some(after) => (true, after.trim_start()),
            None => (false, trimmed),
        };
        let (had_conjunction, next) = ["or ", "and ", "nor "]
            .iter()
            .find_map(|conjunction| next.strip_prefix(conjunction))
            .map_or((false, next), |after| (true, after));
        if !had_comma && !had_conjunction {
            return if skipped_any { cursor } else { rest };
        }
        match COORDINATED_VERBS
            .iter()
            .find_map(|verb| word_prefix(next, verb))
        {
            Some(tail) => {
                cursor = tail;
                skipped_any = true;
            }
            None => return if skipped_any { cursor } else { rest },
        }
    }
}

/// Where the clause holding the object ends: at a comma, semicolon, line break, dash or opening
/// parenthesis, and at a sentence stop or colon that is followed by whitespace, so the dot of
/// `code.py` does not.
fn clause_end(text: &str) -> usize {
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        let next = chars.peek().map(|&(_, next)| next);
        let ends = match c {
            '\n' | ';' | ',' | '(' | '\u{2014}' | '\u{2013}' => true,
            '.' | '!' | '?' | ':' => {
                next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | '"' | '\'' | '*' | '`'))
            }
            _ => false,
        };
        if ends {
            return at;
        }
    }
    text.len()
}

/// Whether the line after a bare "do not edit" starts a list of what is protected.
fn next_line_is_list_item(after_break: &str) -> bool {
    let line = after_break.trim_start();
    let mut chars = line.chars();
    match chars.next() {
        Some('-' | '*' | '\u{2022}' | '>' | '`') => true,
        Some(digit) if digit.is_ascii_digit() => chars
            .find(|c| !c.is_ascii_digit())
            .is_some_and(|c| matches!(c, '.' | ')')),
        _ => false,
    }
}

/// Whether what follows turns from the ban to answering ("just explain", "then stop", "and report")
/// or to a further instruction of its own ("don't run anything either").
fn closing_continuation(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let Some(first) = words.next() else {
        return true;
    };
    let second = words.next().map(bare);
    match bare(first) {
        "just" | "only" | "simply" => second.is_none_or(|word| ACTIONS.contains(&word)),
        "and" | "or" | "nor" | "but" => second.is_some_and(|word| {
            ACTIONS.contains(&word)
                || matches!(
                    word,
                    "then"
                        | "just"
                        | "only"
                        | "simply"
                        | "please"
                        | "do"
                        | "don't"
                        | "dont"
                        | "never"
                        | "no"
                )
        }),
        "make" => second == Some("no"),
        "do" => second == Some("not"),
        "then" | "so" | "instead" | "rather" | "while" | "until" | "unless" | "before"
        | "because" | "since" | "as" | "please" | "thanks" | "thx" | "ok" | "okay"
        | "otherwise" | "no" | "never" | "don't" | "dont" => true,
        word => ACTIONS.contains(&word),
    }
}

/// Whether what follows a clause that ended on a universal object keeps the ban universal: not
/// "..., except `CHANGELOG.md`" or "(only ANSWER.md may change)".
fn ban_survives(after: &str) -> bool {
    let Some(boundary) = after.chars().next() else {
        return true;
    };
    if !matches!(boundary, ',' | '(' | '\u{2014}' | '\u{2013}') {
        return true;
    }
    let next = after[boundary.len_utf8()..].trim_start();
    let Some(first) = next.split_whitespace().next() else {
        return true;
    };
    let first = bare(first);
    if NARROWING_AFTER_BOUNDARY.contains(&first) {
        return false;
    }
    match first {
        "just" | "only" | "simply" | "but" | "and" | "or" | "nor" => closing_continuation(next),
        _ => true,
    }
}

/// `tail` without the fillers that restate a ban, and whether it had any.
fn strip_fillers(tail: &str) -> (&str, bool) {
    let mut rest = tail.trim_start();
    let mut stripped = false;
    while let Some(after) = FILLERS
        .iter()
        .filter_map(|filler| word_prefix(rest, filler))
        .min_by_key(|after| after.len())
    {
        rest = after.trim_start();
        stripped = true;
    }
    (rest, stripped)
}

/// What a universal object is, and what follows it.
struct Matched<'a> {
    tail: &'a str,
    weak: bool,
}

/// The universal object `object` opens with, after a determiner. "any file" is universal, "the
/// file" points at one file, so singular `file` needs an indefinite determiner.
fn universal_object<'a>(object: &'a str, files_only: bool) -> Option<Matched<'a>> {
    let (object, indefinite) = ["any ", "all ", "every ", "each "]
        .iter()
        .find_map(|det| object.strip_prefix(det))
        .map_or((object, false), |rest| (rest.trim_start(), true));
    let object = if indefinite {
        object
            .strip_prefix("of the ")
            .or_else(|| object.strip_prefix("of "))
            .unwrap_or(object)
            .trim_start()
    } else {
        object.strip_prefix("the ").unwrap_or(object).trim_start()
    };
    if indefinite {
        if let Some(tail) = word_prefix(object, "file") {
            return Some(Matched { tail, weak: false });
        }
    }
    if let Some(tail) = word_prefix(object, "files") {
        return Some(Matched { tail, weak: false });
    }
    if files_only {
        return None;
    }
    if let Some(tail) = word_prefix(object, "anything") {
        return Some(Matched { tail, weak: false });
    }
    WEAK_OBJECTS
        .iter()
        .find_map(|name| word_prefix(object, name))
        .or_else(|| (!indefinite).then(|| word_prefix(object, "it")).flatten())
        .map(|tail| Matched { tail, weak: true })
}

/// Whether the prohibition forbids writing anywhere. `change_requested` is evaluated only when the
/// object is one a change request can legitimately end with ("do not touch the code").
fn prohibition_is_universal(
    prohibition: &Prohibition<'_>,
    change_requested: &mut impl FnMut() -> bool,
) -> bool {
    let rest = after_coordinated_verbs(prohibition.rest).trim_start_matches(' ');
    let rest = if prohibition.takes_to {
        word_prefix(rest, "to").map_or(rest, str::trim_start)
    } else {
        rest
    };
    if let Some(dash) = rest.strip_prefix('-') {
        return closing_continuation(dash.trim_start_matches('-'));
    }
    let end = clause_end(rest);
    let object = rest[..end].trim();
    let after = &rest[end..];
    if object.is_empty() {
        return match after.chars().next() {
            Some(':') => false,
            Some('\n') => !next_line_is_list_item(after),
            Some(boundary @ (',' | '(' | '\u{2014}' | '\u{2013}')) => {
                closing_continuation(&after[boundary.len_utf8()..])
            }
            _ => true,
        };
    }
    let Some(mut matched) = universal_object(object, prohibition.files_only) else {
        return BARE_FILLERS.iter().any(|filler| {
            word_prefix(object, filler).is_some_and(|rest| strip_fillers(rest).0.is_empty())
        }) && ban_survives(after);
    };
    while let Some(next) = ["or ", "and ", "nor "]
        .iter()
        .find_map(|conjunction| matched.tail.trim_start().strip_prefix(conjunction))
        .and_then(|rest| universal_object(rest.trim_start(), prohibition.files_only))
    {
        matched = Matched {
            tail: next.tail,
            weak: matched.weak && next.weak,
        };
    }
    let (tail, stripped) = strip_fillers(matched.tail);
    let closes = if let Some(dash) = tail.strip_prefix('-') {
        closing_continuation(dash.trim_start_matches('-'))
    } else {
        tail.is_empty() || closing_continuation(tail)
    };
    if !closes || !ban_survives(after) {
        return false;
    }
    if matched.weak && !stripped && !closes_the_sentence(tail, after) {
        return !change_requested();
    }
    true
}

/// Whether text after a weak object turns from the ban to answering.
fn closes_the_sentence(tail: &str, after: &str) -> bool {
    if !tail.is_empty() {
        return true;
    }
    after
        .chars()
        .next()
        .filter(|c| matches!(c, ',' | '\u{2014}' | '\u{2013}'))
        .is_some_and(|c| {
            closing_continuation(&after[c.len_utf8()..])
                && !after[c.len_utf8()..].trim_start().is_empty()
        })
}
