//! Session recap, suggestion, and project-memory text policy.
//!
//! These deterministic normalizers protect presenter contracts and avoid paid
//! side calls from changing task completion truth.

use std::path::Path;

use forge_types::TodoItem;

/// Reduce a recap completion to the single line the Recap event contract promises. A misbehaving
/// trivial-tier model can ignore the "one sentence" instruction and dump whole paragraphs (or its
/// chain of thought) — clamp to the first non-empty line and a sane length so the scrollback
/// recap stays a recap. `None` when the completion had no usable text at all.
pub(crate) fn recap_line(content: &str) -> Option<String> {
    let line = content.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.chars().take(240).collect())
}

/// Produce a recap from Forge's own task state when this turn moved a tracked plan to fully done.
/// This is deliberately deterministic: an auxiliary summarizer once inverted an explicit
/// "confirmed complete" response into "tasks were not completed". Only use the task state when it
/// changed during this turn and the final answer independently reports completion, preventing an
/// unrelated follow-up from inheriting a stale completed plan's recap.
pub(crate) fn completed_tasks_recap(
    before: &[TodoItem],
    after: &[TodoItem],
    final_text: &str,
) -> Option<String> {
    if after.is_empty()
        || before == after
        || after
            .iter()
            .any(|task| task.status != forge_types::TodoStatus::Done)
    {
        return None;
    }

    let final_lower = final_text.to_ascii_lowercase();
    const NEGATIVE_COMPLETION: &[&str] = &[
        "not complete",
        "not completed",
        "did not complete",
        "didn't complete",
        "could not complete",
        "couldn't complete",
        "failed to complete",
        "unable to complete",
        "incomplete",
        "unfinished",
    ];
    if NEGATIVE_COMPLETION
        .iter()
        .any(|marker| final_lower.contains(marker))
    {
        return None;
    }
    const POSITIVE_COMPLETION: &[&str] = &[
        "confirmed complete",
        "successfully completed",
        "completed all",
        "all done",
        "tasks complete",
        "task complete",
        "tasks done",
        "task done",
    ];
    if !POSITIVE_COMPLETION
        .iter()
        .any(|marker| final_lower.contains(marker))
    {
        return None;
    }

    Some(match after.len() {
        1 => "Completed the tracked task".to_string(),
        count => format!("Completed all {count} tracked tasks"),
    })
}

/// Reduce a next-prompt-suggestion completion to a clean ghost-text candidate: the first
/// non-empty line, with quote/backtick characters and any embedded newlines stripped, capped at
/// 160 chars. `None` when the result is empty, or when it's just the prompt that was already run
/// (case-insensitive, trimmed) — a suggestion that repeats what the user just asked for is
/// useless, and a misbehaving trivial-tier model doing that is more likely than it seems.
pub(crate) fn sanitize_suggestion(content: &str, prev_prompt: &str) -> Option<String> {
    let line = content.lines().map(str::trim).find(|l| !l.is_empty())?;
    let cleaned: String = line
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '`' | '\n' | '\r'))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.eq_ignore_ascii_case(prev_prompt.trim()) {
        return None;
    }
    Some(cleaned.chars().take(160).collect())
}

/// Scope key for auto-memory: the current project directory's absolute path (memories are
/// per-project). Matches the `forge memory` CLI so both see the same store.
pub(crate) fn memory_scope_at(root: &Path) -> String {
    root.display().to_string()
}

/// Drop leading blank lines from a completion note before it's shown/persisted. A bridge's final
/// wrap-up text sometimes opens with one or more empty lines (formatting artifacts of its own
/// output), which the `/api/history` non-tool view then rendered as a note that visibly starts
/// with dead space (#28: "\n\nAll tasks complete. ..."). Only LEADING blanks are removed —
/// intentional blank lines further into the note are left alone.
pub(crate) fn trim_leading_blank_lines(content: &str) -> &str {
    let mut rest = content;
    while let Some(after_newline) = rest
        .split_once('\n')
        .filter(|(line, _)| line.trim().is_empty())
        .map(|(_, after)| after)
    {
        rest = after_newline;
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::trim_leading_blank_lines;

    #[test]
    fn trim_leading_blank_lines_drops_only_the_leading_run() {
        assert_eq!(
            trim_leading_blank_lines("\n\nAll tasks complete. Everything shipped."),
            "All tasks complete. Everything shipped."
        );
        assert_eq!(
            trim_leading_blank_lines("  \n\t\n\nDone."),
            "Done.",
            "whitespace-only lines count as blank, not just empty ones"
        );
        assert_eq!(
            trim_leading_blank_lines("No leading blanks here."),
            "No leading blanks here."
        );
        assert_eq!(
            trim_leading_blank_lines("First line.\n\nSecond paragraph."),
            "First line.\n\nSecond paragraph.",
            "a blank line further in is intentional formatting, not dead space to strip"
        );
        assert_eq!(
            trim_leading_blank_lines("\n\n"),
            "",
            "an all-blank note collapses to empty, same as one with nothing to say"
        );
    }
}
