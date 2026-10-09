//! Detects a final reply that announces the next action and then stops without doing it.
//!
//! Real sessions show the pattern constantly: the model writes "Now the worker side..." or "Let me
//! check the log:" with no tool call, the turn ends, and the user has to type `continue`. Nothing
//! else catches it when no task list is open. The classifier looks only at the LAST sentence of the
//! reply, because that is where an abandoned intention lives; earlier prose is usually a summary.

/// Forward-looking phrases that start (or sit inside) a sentence announcing agent work.
const INTENT_CLAUSES: &[&str] = &[
    "let me ",
    "let's ",
    "let us ",
    "i'll ",
    "i will ",
    "i'm going to ",
    "i am going to ",
    "i'm about to ",
    "i need to ",
    "i should ",
    "i must ",
    "i want to ",
    "we need to ",
    "we should ",
];

/// Openers that announce work without a first-person subject ("Now the worker side...").
const INTENT_OPENERS: &[&str] = &[
    "now the ",
    "now for ",
    "now to ",
    "now i ",
    "now we ",
    "now let",
    "now,",
    "next,",
    "next i ",
    "next we ",
    "next up",
    "next: ",
    "moving on to ",
    "moving to ",
    "time to ",
    "starting with ",
    "first, i ",
    "first i ",
    "first, let",
    "to do this",
    "to verify",
    "to check",
    "to confirm",
    "i'm now ",
    "i am now ",
];

/// Action gerunds that read as "I am doing this right now" when they open the last sentence.
const ACTION_GERUNDS: &[&str] = &[
    "checking",
    "building",
    "writing",
    "running",
    "reading",
    "looking",
    "searching",
    "fixing",
    "updating",
    "adding",
    "creating",
    "implementing",
    "decoding",
    "wiring",
    "applying",
    "pulling",
    "fetching",
    "testing",
    "verifying",
    "inspecting",
    "reviewing",
    "investigating",
    "examining",
    "trying",
    "starting",
    "launching",
    "rerunning",
    "re-running",
    "restructuring",
    "pinning",
    "locating",
    "mapping",
    "extracting",
    "editing",
    "committing",
    "pushing",
    "merging",
    "rebasing",
    "installing",
    "compiling",
    "tracing",
    "patching",
    "finishing",
    "continuing",
    "grepping",
    "querying",
    "scanning",
    "loading",
    "opening",
    "reapplying",
    "retrying",
    "rewriting",
    "moving",
    "removing",
    "deleting",
    "generating",
    "setting",
    "making",
    "doing",
    "digging",
    "diffing",
    "listing",
    "comparing",
    "reproducing",
    "validating",
    "confirming",
    "capturing",
];

/// A sentence that hands the next move to the user, or only offers more work, is a legitimate end.
const USER_DIRECTED: &[&str] = &[
    "if you want",
    "if you'd like",
    "if you would like",
    "if you prefer",
    "if you need",
    "would you like",
    "do you want",
    "shall i",
    "should i",
    "want me to",
    "let me know",
    "say the word",
    "just say",
    "your call",
    "tell me",
    "point me",
    "give me",
    "send me",
    "paste ",
    "once you",
    "when you",
    "after you",
    "until you",
    "as soon as you",
    "feel free",
    "ready when you",
    "standing by",
    "awaiting",
    "waiting on you",
    "waiting for you",
    "waiting for your",
    "waiting on your",
    "i can also",
    "i can do",
    "i'm happy to",
    "i am happy to",
    "i'd be happy",
    "i'll leave",
    "i will leave",
    "i'll stop",
    "i will stop",
    "i'll wait",
    "i will wait",
    "i'll hold",
    "i'll defer",
    "i'll need you",
    "i need you",
    "i need your",
    "i need clarification",
    "i need access",
    "i need permission",
    "need your",
    "need you to",
    "need from you",
    "say when",
    "in mind",
    "ping me",
    "share it",
    "your approval",
    "request changes",
    "check in",
    "report back",
    "i'll report",
    "i'll help",
    "i will help",
    "i'll get to work",
];

/// Present-perfect and stative openers after "now" that describe a state, not a next action.
const NOW_STATIVE: &[&str] = &[
    "now i have",
    "now i see",
    "now i understand",
    "now i know",
    "now i can see",
    "now i get",
    "now i'm clear",
    "now i am clear",
    "now it ",
    "now they ",
    "now there",
    "now that",
    "now the tests pass",
    "now the build passes",
];

/// Whether the reply's last sentence announces work the model has not done.
pub(crate) fn ends_mid_intent(text: &str) -> bool {
    let Some((sentence, paragraph)) = last_sentence(text) else {
        return false;
    };
    let s = sentence.to_lowercase().replace('’', "'");
    // The hand-off may sit in an earlier sentence of the paragraph ("Point me at it. Then I'll
    // implement it."), so the whole last paragraph is checked for user-directed phrasing.
    let paragraph = paragraph.to_lowercase().replace('’', "'");
    if s.contains('?') || USER_DIRECTED.iter().any(|m| paragraph.contains(m)) {
        return false;
    }
    if NOW_STATIVE.iter().any(|m| s.starts_with(m)) {
        return false;
    }
    let s = s.trim_start_matches(|c: char| !c.is_alphanumeric());
    let lead = strip_lead_in(s);
    if INTENT_OPENERS
        .iter()
        .any(|o| lead.starts_with(o) || s.starts_with(o))
    {
        return !is_report_of_state(lead) && !describes_state(lead);
    }
    if INTENT_CLAUSES.iter().any(|c| contains_clause(s, c)) {
        return true;
    }
    if let Some(first) = lead
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .next()
    {
        if ACTION_GERUNDS.contains(&first) {
            return true;
        }
    }
    // "I'm checking ...", "now I'm pinning ...": first-person progressive.
    for prefix in ["i'm ", "i am "] {
        let mut from = 0;
        while let Some(at) = s[from..].find(prefix) {
            let rest = &s[from + at + prefix.len()..];
            let rest = rest.strip_prefix("now ").unwrap_or(rest);
            let word = rest
                .split(|c: char| !(c.is_alphanumeric() || c == '-'))
                .next()
                .unwrap_or("");
            if ACTION_GERUNDS.contains(&word) {
                return true;
            }
            from += at + prefix.len();
        }
    }
    false
}

/// "Now the track is bg2 ..." reports a result; "Now the worker side..." announces work.
fn describes_state(lead: &str) -> bool {
    lead.starts_with("now the ")
        && [
            " is ", " are ", " was ", " were ", " has ", " have ", " now ", " passes", " works",
        ]
        .iter()
        .any(|v| lead.contains(v))
}

/// `now i ...` / `next, ...` openers still describe a finished state sometimes ("Now I'm done").
fn is_report_of_state(lead: &str) -> bool {
    ["done", "finished", "complete", "all set", "ready"]
        .iter()
        .any(|w| {
            lead.starts_with(&format!("now i'm {w}")) || lead.starts_with(&format!("now i am {w}"))
        })
}

/// A clause match that is not buried in a quotation or part of another word.
fn contains_clause(s: &str, clause: &str) -> bool {
    let mut from = 0;
    while let Some(at) = s[from..].find(clause) {
        let start = from + at;
        let boundary = start == 0
            || s[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric());
        if boundary {
            return true;
        }
        from = start + clause.len();
    }
    false
}

/// Drops conversational lead-ins so "Good, now let me ..." is judged like "Now let me ...".
fn strip_lead_in(s: &str) -> &str {
    let mut s = s;
    for _ in 0..3 {
        let before = s;
        for lead in [
            "ok, ",
            "okay, ",
            "ok ",
            "okay ",
            "good, ",
            "good. ",
            "great, ",
            "alright, ",
            "right, ",
            "so, ",
            "so ",
            "and ",
            "but ",
            "also, ",
            "perfect, ",
            "nice, ",
            "done. ",
            "found it. ",
        ] {
            if let Some(rest) = s.strip_prefix(lead) {
                s = rest;
            }
        }
        if s == before {
            break;
        }
    }
    s
}

fn is_numbered_item(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with([')', '.']) && line[digits + 1..].starts_with(' ')
}

/// The final sentence of the final paragraph, or `None` when the reply ends in something that is
/// not prose (a code fence, a list item, a table row, a heading).
fn last_sentence(text: &str) -> Option<(String, String)> {
    let trimmed = text
        .trim()
        .trim_end_matches([':', '—', '–', '-', '…', '.', ' ', '\n']);
    if trimmed.is_empty() || trimmed.ends_with("```") {
        return None;
    }
    let paragraph = trimmed.rsplit("\n\n").next()?;
    let line = paragraph.lines().next_back()?.trim();
    let whole = paragraph.to_string();
    if line.starts_with(['-', '*', '|', '>', '#']) || is_numbered_item(line) {
        return None;
    }
    // Sentence boundary: terminator followed by whitespace. File names such as `lib.rs` and
    // version numbers never have a space after the dot.
    let bytes = line.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?' | b';') && bytes.get(i + 1).is_some_and(|n| *n == b' ') {
            let rest = line[i + 1..].trim_start();
            if !rest.is_empty() {
                start = line.len() - rest.len();
            }
        }
    }
    let sentence = line[start..].trim();
    // A dangling fragment ("Let me:") borrows the sentence before it.
    if sentence.split_whitespace().count() < 3 && start > 0 {
        let head = line[..start].trim_end();
        let head_start = head.rfind(['.', '!', '?']).map_or(0, |i| i + 1);
        return Some((format!("{} {}", head[head_start..].trim(), sentence), whole));
    }
    Some((sentence.to_string(), whole))
}
