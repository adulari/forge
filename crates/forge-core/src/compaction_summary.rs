//! What a compaction summary says and how it is checked: the prompt, stripping the model's scratch
//! section, the structural check, and the user's own words that the harness appends verbatim.
//!
//! Pure text rules only; the model calls live in `compaction_run`.

use crate::tokens;

/// Opens the system message a compaction leaves in place of the folded messages. The store builds
/// the same prefix when a resumed session loads its summary row.
pub(crate) const SUMMARY_PREFIX: &str = "[Earlier conversation summarized to save context]";

pub(crate) const COMPACT_SYSTEM: &str =
    "You are writing the working memory of a coding agent. The \
conversation below is about to be removed from its context; your summary is ALL it will remember \
of it, so a detail you leave out is lost for good. Do not continue the task, do not call tools, \
and do not address the user — only write the summary.\n\n\
First write an <analysis> section as a scratchpad; it is thrown away. Walk through the \
conversation chronologically and, for each part, note: what the user asked for and why; what you \
did about it; the decisions made and the technical concepts involved; the exact files, function \
signatures, and code you read or changed; the errors you hit and how each was fixed; and any \
feedback or correction the user gave, especially where they told you to do something differently. \
Check every point against the conversation for accuracy and completeness.\n\n\
Then write the summary inside <summary></summary> tags, as markdown with these numbered \
sections, in this order:\n\
1. Primary request and intent — everything the user asked for, in their own words where it \
matters, including constraints and preferences they stated.\n\
2. Key technical concepts — the technologies, frameworks, and ideas the work rests on.\n\
3. Files and code — every file path read, changed, or found relevant: why it matters, what \
changed in it, and short snippets of the code that was written or edited.\n\
4. Errors and fixes — each failure seen, how it was fixed, and every correction or complaint the \
user made about how you were working (being told you were looping or repeating yourself, doing \
the wrong thing, ignoring an instruction).\n\
5. Problem solving — what was solved, what is still being investigated, approaches chosen and \
rejected, and why.\n\
6. Pending tasks — what you were asked to do that is not done yet.\n\
7. Current work — precisely what was being worked on immediately before this point, with file \
names and code snippets, and what you had already answered or tried so it is not answered or \
tried again.\n\
8. Next step — only a step that directly follows from the user's latest request and the current \
work, with a verbatim quote of the latest instruction so the task cannot drift. Leave it out if \
the last task was finished.\n\n\
Be specific: exact names, paths, numbers, and error text beat paraphrase. Scale the length to the \
conversation — a long session needs a long summary; do not compress hours of work into a \
paragraph. Do not copy the user's messages out in full: they are appended verbatim after your \
summary.";

/// Added to the system prompt when the conversation reaches the model in parts.
const COMPACT_PARTS_NOTE: &str = "The conversation is too long for one request, so it arrives in \
parts, oldest first. When <previous_summary> is present it is the summary of everything before \
the part you are given; write the complete updated summary of the previous summary AND this part, \
in the same format. Keep every detail of the previous summary unless this part supersedes it. \
Only the final summary is kept, so each reply must stand on its own.";

/// The system prompt for one summarizer call. `multi_part` is set for any call of a rolling
/// summary, including its first.
pub(crate) fn system_prompt(directive: Option<&str>, multi_part: bool) -> String {
    let mut out = COMPACT_SYSTEM.to_string();
    if multi_part {
        out.push_str("\n\n");
        out.push_str(COMPACT_PARTS_NOTE);
    }
    if let Some(directive) = directive {
        out.push_str("\n\n");
        out.push_str(directive);
    }
    out
}

/// The `/compact <focus>` instruction, appended to the prompt like a reground directive.
pub(crate) fn focus_directive(focus: &str) -> String {
    format!(
        "The user asked for this compaction with these instructions. Follow them when deciding \
         what to keep and emphasize; they do not change the section structure:\n{}",
        focus.trim()
    )
}

/// The user message of one summarizer call: the conversation part, preceded by the summary so far
/// when the transcript is summarized in parts.
pub(crate) fn rolling_payload(previous: Option<&str>, part: &str, is_last: bool) -> String {
    match (previous, is_last) {
        (None, true) => part.to_string(),
        (None, false) => format!(
            "This is the first part of the conversation; more parts follow.\n\n\
             <conversation_part>\n{part}\n</conversation_part>"
        ),
        (Some(previous), is_last) => {
            let position = if is_last {
                "This is the final part of the conversation."
            } else {
                "More parts follow this one."
            };
            format!(
                "<previous_summary>\n{previous}\n</previous_summary>\n\n{position}\n\n\
                 <conversation_part>\n{part}\n</conversation_part>"
            )
        }
    }
}

/// The text between `<summary>` tags, or the whole reply when the tags are missing. The
/// `<analysis>` scratch section never survives: it is cut out even when the model skipped the
/// summary tags.
pub(crate) fn extract_summary(reply: &str) -> String {
    let text = reply.trim();
    let lower = text.to_ascii_lowercase();
    if let Some(open) = lower.find("<summary>") {
        let start = open + "<summary>".len();
        let end = lower[start..]
            .find("</summary>")
            .map_or(text.len(), |close| start + close);
        return text[start..end].trim().to_string();
    }
    if let (Some(open), Some(close)) = (lower.find("<analysis>"), lower.find("</analysis>")) {
        if open < close {
            let mut out = String::from(&text[..open]);
            out.push_str(&text[close + "</analysis>".len()..]);
            return out.trim().to_string();
        }
    }
    text.to_string()
}

/// Whether `summary` looks like the sectioned summary the prompt asks for: at least three of its
/// numbered sections or at least three markdown headings. Counts markers instead of looking for
/// English words, so a summary written in the user's language passes.
pub(crate) fn has_summary_structure(summary: &str) -> bool {
    let mut numbered = [false; 8];
    let mut headings = 0usize;
    for line in summary.lines() {
        let line = line.trim_start();
        if line.starts_with('#') {
            headings += 1;
        }
        let rest = line.trim_start_matches(['#', '*', ' ']);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let after = &rest[digits.len()..];
        if !digits.is_empty() && after.starts_with(['.', ')', ':']) {
            if let Ok(n) = digits.parse::<usize>() {
                if (1..=8).contains(&n) {
                    numbered[n - 1] = true;
                }
            }
        }
    }
    numbered.iter().filter(|seen| **seen).count() >= 3 || headings >= 3
}

/// Largest slice of a tool result or injected note the summarizer reads, head and tail kept.
pub(crate) const INPUT_TOOL_RESULT_MAX_CHARS: usize = 12_000;
/// Largest slice of any other message the summarizer reads.
pub(crate) const INPUT_MESSAGE_MAX_CHARS: usize = 24_000;

/// `text` cut to `max` chars with its start and end kept and the cut said out loud.
pub(crate) fn head_tail(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let head = max / 2;
    let tail = max - head;
    let head_text: String = text.chars().take(head).collect();
    let tail_text: String = text.chars().skip(total - tail).collect();
    format!(
        "{head_text}\n[… {} chars omitted from the middle …]\n{tail_text}",
        total - max
    )
}

/// Heading of the harness-built section holding the user's own words.
pub(crate) const USER_MESSAGES_HEADING: &str = "## User messages (verbatim)";
const USER_MESSAGES_INTRO: &str =
    "Added by the harness from the conversation itself, oldest first; these are the user's exact \
     words, not a paraphrase.";
/// Token budget of the whole section.
pub(crate) const USER_MESSAGES_BUDGET_TOKENS: usize = 8_000;
/// Longest an older message is kept; the newest is kept whole.
pub(crate) const USER_MESSAGE_OLDER_MAX_CHARS: usize = 2_000;

/// One user message in the verbatim section. `number` counts the session's folded user messages
/// from 1 across every compaction, so a later compaction can tell how many were dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerbatimEntry {
    pub number: usize,
    pub text: String,
}

fn marker(number: usize) -> String {
    format!("[user message #{number}]")
}

fn parse_marker(line: &str) -> Option<usize> {
    line.trim()
        .strip_prefix("[user message #")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// A summary split into what the model wrote and the verbatim section a previous compaction
/// appended to it.
pub(crate) fn split_verbatim(summary: &str) -> (&str, Vec<VerbatimEntry>) {
    let opening = format!("{USER_MESSAGES_HEADING}\n{USER_MESSAGES_INTRO}");
    let Some(start) = summary.find(&opening) else {
        return (summary, Vec::new());
    };
    let body = summary[..start].trim_end();
    let block = &summary[start + opening.len()..];
    let mut entries: Vec<VerbatimEntry> = Vec::new();
    let mut current: Option<(usize, Vec<&str>)> = None;
    let mut last: Option<usize> = None;
    let finish = |current: Option<(usize, Vec<&str>)>, entries: &mut Vec<VerbatimEntry>| {
        if let Some((number, lines)) = current {
            entries.push(VerbatimEntry {
                number,
                text: lines.join("\n").trim().to_string(),
            });
        }
    };
    for line in block.lines() {
        // Markers are numbered consecutively, so a line inside a user's text that merely looks
        // like one is told apart by its number.
        match parse_marker(line).filter(|number| last.is_none_or(|prev| *number == prev + 1)) {
            Some(number) => {
                finish(current.take(), &mut entries);
                current = Some((number, Vec::new()));
                last = Some(number);
            }
            None => {
                if let Some((_, lines)) = current.as_mut() {
                    lines.push(line);
                }
            }
        }
    }
    finish(current.take(), &mut entries);
    (body, entries)
}

fn cap_chars(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n[… {} more chars not shown]", total - max)
}

/// The harness-built verbatim section: `previous` entries (from the summary this compaction
/// absorbs) followed by `new` messages, oldest first. The newest message is kept whole, older
/// ones are capped, and the whole section stays inside [`USER_MESSAGES_BUDGET_TOKENS`] by
/// dropping the oldest, with the count said. Empty when there are no messages at all.
pub(crate) fn render_verbatim_block(previous: &[VerbatimEntry], new: &[String]) -> String {
    let mut all: Vec<VerbatimEntry> = previous.to_vec();
    let first_new = all.last().map_or(0, |e| e.number) + 1;
    for (number, text) in (first_new..).zip(new) {
        all.push(VerbatimEntry {
            number,
            text: text.trim().to_string(),
        });
    }
    if all.is_empty() {
        return String::new();
    }
    let budget = USER_MESSAGES_BUDGET_TOKENS;
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut first_kept = all.last().map_or(1, |e| e.number);
    for (index, entry) in all.iter().enumerate().rev() {
        let is_newest = index == all.len() - 1;
        let text = if is_newest {
            fit_whole(&entry.text, budget)
        } else {
            cap_chars(&entry.text, USER_MESSAGE_OLDER_MAX_CHARS)
        };
        let rendered = format!("{}\n{text}", marker(entry.number));
        let cost = tokens::count_text(&rendered) + 2;
        if !is_newest && used + cost > budget {
            break;
        }
        used += cost;
        first_kept = entry.number;
        kept.push(rendered);
    }
    kept.reverse();
    let dropped_before = first_kept.saturating_sub(1);
    let mut out = format!("{USER_MESSAGES_HEADING}\n{USER_MESSAGES_INTRO}\n");
    if dropped_before > 0 {
        out.push_str(&format!(
            "[The {dropped_before} oldest user message(s) are not shown: they no longer fit this \
             section.]\n"
        ));
    }
    out.push_str(&kept.join("\n\n"));
    out
}

/// `text` kept whole unless it alone exceeds `budget_tokens`, in which case its start and end are
/// kept.
fn fit_whole(text: &str, budget_tokens: usize) -> String {
    if tokens::count_text(text) <= budget_tokens {
        return text.to_string();
    }
    // About four characters per token to start with; dense text needs more cutting.
    let mut max = budget_tokens.saturating_sub(64) * 4;
    loop {
        let cut = head_tail(text, max);
        if tokens::count_text(&cut) <= budget_tokens.saturating_sub(64) || max < 256 {
            return cut;
        }
        max = max * 3 / 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analysis_scratchpad_is_stripped_and_the_summary_kept() {
        let reply = "<analysis>\nthe user wanted X; I did Y\n</analysis>\n\n<summary>\n1. Primary \
                     request\nfix the parser\n</summary>";
        assert_eq!(extract_summary(reply), "1. Primary request\nfix the parser");
    }

    #[test]
    fn a_reply_without_tags_is_used_whole() {
        assert_eq!(extract_summary("  1. Goal\nship it  "), "1. Goal\nship it");
    }

    #[test]
    fn analysis_without_summary_tags_is_still_cut_out() {
        let reply = "<analysis>scratch</analysis>\n1. Request\nship it";
        let out = extract_summary(reply);
        assert!(!out.contains("scratch") && out.contains("ship it"), "{out}");
    }

    #[test]
    fn an_unclosed_summary_tag_keeps_what_was_written() {
        let out = extract_summary("<analysis>a</analysis><summary>\n1. Request\ncut off here");
        assert_eq!(out, "1. Request\ncut off here");
    }

    #[test]
    fn structure_is_counted_from_markers_not_english_words() {
        assert!(has_summary_structure(
            "1. Verzoek\n2. Technische concepten\n3. Bestanden en code\ntekst"
        ));
        assert!(has_summary_structure("## A\nx\n## B\ny\n## C\nz"));
        assert!(has_summary_structure(
            "**1. Request**\nx\n### 4) Errors\ny\n5: Problem\nz"
        ));
        assert!(!has_summary_structure(
            "1. Request\nthe user asked for a lot of things"
        ));
        assert!(!has_summary_structure(
            &"the goal of the session ".repeat(100)
        ));
    }

    #[test]
    fn the_prompt_names_every_section_and_the_scratchpad() {
        for needle in [
            "<analysis>",
            "<summary>",
            "Primary request and intent",
            "Key technical concepts",
            "Files and code",
            "Errors and fixes",
            "Problem solving",
            "Pending tasks",
            "Current work",
            "Next step",
            "verbatim quote",
            "do not continue the task",
        ] {
            assert!(
                COMPACT_SYSTEM
                    .to_lowercase()
                    .contains(&needle.to_lowercase()),
                "{needle}"
            );
        }
    }

    #[test]
    fn head_tail_keeps_both_ends_and_says_how_much_is_gone() {
        let text = format!("{}{}", "a".repeat(1_000), "z".repeat(1_000));
        let cut = head_tail(&text, 100);
        assert!(cut.starts_with(&"a".repeat(50)) && cut.ends_with(&"z".repeat(50)));
        assert!(cut.contains("1900 chars omitted"));
        assert_eq!(head_tail("short", 100), "short");
    }

    #[test]
    fn the_verbatim_block_keeps_the_newest_whole_and_caps_older_messages() {
        let long = "x".repeat(5_000);
        let block = render_verbatim_block(&[], &["first".into(), long.clone(), long.clone()]);
        assert!(block.starts_with(USER_MESSAGES_HEADING));
        assert!(block.contains("[user message #1]\nfirst"));
        assert!(block.contains("more chars not shown"), "older capped");
        assert!(
            block.ends_with(&long),
            "the newest message is kept whole, not capped"
        );
    }

    #[test]
    fn the_verbatim_block_drops_the_oldest_to_fit_and_counts_them() {
        let messages: Vec<String> = (0..60)
            .map(|i| format!("request {i} {}", "word ".repeat(380)))
            .collect();
        let block = render_verbatim_block(&[], &messages);
        assert!(
            tokens::count_message(&block) <= USER_MESSAGES_BUDGET_TOKENS + 200,
            "{} tokens",
            tokens::count_message(&block)
        );
        assert!(block.contains("request 59 "), "newest survives");
        assert!(!block.contains("request 0 "), "oldest is dropped first");
        let (_, kept) = split_verbatim(&block);
        let dropped = 60 - kept.len();
        assert!(dropped > 0);
        assert!(block.contains(&format!("The {dropped} oldest user message(s)")));
        assert_eq!(kept.last().unwrap().number, 60);
    }

    #[test]
    fn an_oversized_newest_message_is_fitted_to_the_whole_budget() {
        let huge = format!("START {} END", "word ".repeat(40_000));
        let block = render_verbatim_block(&[], &["old".into(), huge]);
        assert!(tokens::count_message(&block) <= USER_MESSAGES_BUDGET_TOKENS + 200);
        assert!(block.contains("START") && block.contains("END"));
    }

    #[test]
    fn a_block_round_trips_through_split() {
        let block = render_verbatim_block(&[], &["one\nwith two lines".into(), "two".into()]);
        let summary = format!("1. Request\nbody text\n\n{block}");
        let (body, entries) = split_verbatim(&summary);
        assert_eq!(body, "1. Request\nbody text");
        assert_eq!(
            entries,
            vec![
                VerbatimEntry {
                    number: 1,
                    text: "one\nwith two lines".into()
                },
                VerbatimEntry {
                    number: 2,
                    text: "two".into()
                },
            ]
        );
    }

    #[test]
    fn chained_compactions_carry_earlier_user_messages_forward() {
        let first = render_verbatim_block(&[], &["build the parser".into(), "use nom".into()]);
        let summary = format!("1. Request\nparser\n\n{first}");
        let (_, previous) = split_verbatim(&summary);

        let second = render_verbatim_block(&previous, &["now add tests".into()]);
        let (_, entries) = split_verbatim(&second);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["build the parser", "use nom", "now add tests"]);
        assert_eq!(
            entries.iter().map(|e| e.number).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn a_chain_of_compactions_drops_the_oldest_first_and_keeps_counting() {
        let mut summary = String::new();
        for round in 0..8 {
            let (_, previous) = split_verbatim(&summary);
            let messages: Vec<String> = (0..4)
                .map(|i| format!("round {round} message {i} {}", "word ".repeat(300)))
                .collect();
            summary = format!(
                "1. a\n2. b\n3. c\n\n{}",
                render_verbatim_block(&previous, &messages)
            );
        }
        let (_, entries) = split_verbatim(&summary);
        assert_eq!(entries.last().unwrap().number, 32);
        assert!(
            entries.first().unwrap().number > 1,
            "the earliest were dropped"
        );
        assert!(summary.contains("oldest user message(s) are not shown"));
        assert!(
            summary.contains("round 7 message 3 "),
            "the newest is whole"
        );
        assert!(tokens::count_message(&summary) <= USER_MESSAGES_BUDGET_TOKENS + 300);
    }

    #[test]
    fn a_user_line_that_looks_like_a_marker_does_not_split_an_entry() {
        let block = render_verbatim_block(
            &[],
            &[
                "see the list:\n[user message #9]\nnot a real one".into(),
                "next".into(),
            ],
        );
        let (_, entries) = split_verbatim(&block);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].text.contains("not a real one"));
    }

    #[test]
    fn no_user_messages_means_no_block() {
        assert!(render_verbatim_block(&[], &[]).is_empty());
    }

    #[test]
    fn rolling_payload_wraps_only_when_the_transcript_comes_in_parts() {
        assert_eq!(rolling_payload(None, "part", true), "part");
        assert!(rolling_payload(None, "part", false).contains("more parts follow"));
        let with_previous = rolling_payload(Some("SO FAR"), "part", true);
        assert!(with_previous.contains("<previous_summary>\nSO FAR\n</previous_summary>"));
        assert!(with_previous.contains("final part"));
    }

    #[test]
    fn the_prompt_carries_the_focus_and_the_parts_note_when_asked() {
        let plain = system_prompt(None, false);
        assert!(!plain.contains("arrives in parts"));
        let full = system_prompt(Some(&focus_directive("keep the migration plan")), true);
        assert!(full.contains("arrives in parts") && full.contains("keep the migration plan"));
    }
}
