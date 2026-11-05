//! The review draft as plain data, ported from the desktop `reviewDraft.ts` and
//! `lib/reviewComments.ts`: comments pile up per agent, then go out as one prompt ("Send review")
//! or one at a time, and a hunk can be sent as a `/btw` side question ("Explain this change").

/// Context lines kept with a comment, so the agent sees the code without reading the file.
pub const CONTEXT_LINES: usize = 24;
const EXPLAIN_MAX_LINES: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewLine {
    pub line_no: usize,
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: u64,
    pub session_id: String,
    pub path: String,
    pub header: String,
    pub staged: bool,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<ReviewLine>,
    pub text: String,
}

pub fn draft_label(count: usize) -> String {
    if count == 1 {
        "1 comment".to_string()
    } else {
        format!("{count} comments")
    }
}

fn range_label(start: usize, end: usize) -> String {
    if start == end {
        format!("new L{start}")
    } else {
        format!("new L{start}\u{2013}L{end}")
    }
}

fn format_prompt(intro: &str, comments: &[&ReviewComment]) -> String {
    let intro = if intro.trim().is_empty() {
        "Please address the following review feedback."
    } else {
        intro.trim()
    };
    let blocks: Vec<String> = comments
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let bucket = if c.staged { "staged" } else { "working tree" };
            let feedback = c
                .text
                .trim()
                .lines()
                .map(|l| format!("   {l}"))
                .collect::<Vec<_>>()
                .join("\n");
            let context = c
                .lines
                .iter()
                .map(|l| {
                    let gutter = match l.kind {
                        LineKind::Add => '+',
                        LineKind::Del => '-',
                        LineKind::Context => ' ',
                    };
                    format!("    {:>4} {gutter}{}", l.line_no, l.text)
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "{}. `{}` \u{b7} {} \u{b7} {bucket}\n{feedback}\n   Context:\n{}",
                i + 1,
                c.path.replace('`', "\\`"),
                range_label(c.start_line, c.end_line),
                if context.is_empty() {
                    "    (no text context)".to_string()
                } else {
                    context
                }
            )
        })
        .collect();
    format!("{intro}\n\nReview annotations:\n{}", blocks.join("\n\n"))
}

/// One prompt carrying every comment, in the order they were written.
pub fn build_review_prompt(comments: &[ReviewComment]) -> String {
    let refs: Vec<&ReviewComment> = comments.iter().collect();
    format_prompt("Please address this review.", &refs)
}

/// A single comment sent on its own.
pub fn build_single_comment_prompt(comment: &ReviewComment) -> String {
    format_prompt("Please address this review comment.", &[comment])
}

/// The `/btw` side question for "Explain this change": answered without interrupting the turn.
pub fn explain_prompt(path: &str, header: &str, lines: &[String]) -> String {
    let shown = &lines[..lines.len().min(EXPLAIN_MAX_LINES)];
    let cut = if lines.len() > shown.len() {
        format!("\n... {} more lines", lines.len() - shown.len())
    } else {
        String::new()
    };
    format!(
        "/btw Explain this change in {path} ({}). Why was it made and what does it affect?\n\n{}{cut}",
        header.trim(),
        shown.join("\n")
    )
}

/// Comments per agent, in the order they were written. A comment is kept until its review is
/// sent or it is removed.
#[derive(Debug, Default)]
pub struct Drafts {
    by_session: std::collections::HashMap<String, Vec<ReviewComment>>,
    next: u64,
}

impl Drafts {
    pub fn add(&mut self, mut comment: ReviewComment) -> u64 {
        self.next += 1;
        comment.id = self.next;
        let id = comment.id;
        self.by_session
            .entry(comment.session_id.clone())
            .or_default()
            .push(comment);
        id
    }

    pub fn of(&self, session_id: &str) -> &[ReviewComment] {
        self.by_session.get(session_id).map_or(&[], Vec::as_slice)
    }

    pub fn count(&self, session_id: &str) -> usize {
        self.of(session_id).len()
    }

    pub fn count_for(&self, session_id: &str, path: &str, header: &str) -> usize {
        self.of(session_id)
            .iter()
            .filter(|c| c.path == path && c.header == header)
            .count()
    }

    pub fn pop(&mut self, session_id: &str) -> Option<ReviewComment> {
        let list = self.by_session.get_mut(session_id)?;
        let out = list.pop();
        if list.is_empty() {
            self.by_session.remove(session_id);
        }
        out
    }

    /// Take the whole draft for sending.
    pub fn take(&mut self, session_id: &str) -> Vec<ReviewComment> {
        self.by_session.remove(session_id).unwrap_or_default()
    }

    /// Put a draft back (a send that did not go out).
    pub fn restore(&mut self, session_id: &str, mut comments: Vec<ReviewComment>) {
        let list = self.by_session.entry(session_id.to_string()).or_default();
        comments.append(list);
        *list = comments;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(session: &str, path: &str, text: &str) -> ReviewComment {
        ReviewComment {
            id: 0,
            session_id: session.into(),
            path: path.into(),
            header: "@@ -1 +1 @@".into(),
            staged: false,
            start_line: 3,
            end_line: 4,
            lines: vec![
                ReviewLine {
                    line_no: 3,
                    kind: LineKind::Add,
                    text: "let a = 1;".into(),
                },
                ReviewLine {
                    line_no: 4,
                    kind: LineKind::Context,
                    text: "".into(),
                },
            ],
            text: text.into(),
        }
    }

    #[test]
    fn labels_pluralise() {
        assert_eq!(draft_label(1), "1 comment");
        assert_eq!(draft_label(3), "3 comments");
    }

    #[test]
    fn comments_batch_per_agent_in_order() {
        let mut drafts = Drafts::default();
        drafts.add(comment("a", "x.rs", "one"));
        drafts.add(comment("b", "y.rs", "other agent"));
        drafts.add(comment("a", "z.rs", "two"));
        assert_eq!(drafts.count("a"), 2);
        let taken = drafts.take("a");
        assert_eq!(
            taken.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(drafts.count("a"), 0);
        assert_eq!(drafts.count("b"), 1);
    }

    #[test]
    fn one_prompt_numbers_every_comment_with_its_context() {
        let prompt = build_review_prompt(&[
            comment("a", "x.rs", "rename this"),
            comment("a", "z.rs", "line one\nline two"),
        ]);
        assert!(prompt.starts_with("Please address this review.\n\nReview annotations:\n1. `x.rs` \u{b7} new L3\u{2013}L4 \u{b7} working tree\n   rename this\n   Context:\n       3 +let a = 1;"));
        assert!(prompt.contains("2. `z.rs`"));
        assert!(prompt.contains("   line one\n   line two"));
    }

    #[test]
    fn a_single_comment_has_its_own_intro() {
        let prompt = build_single_comment_prompt(&comment("a", "x.rs", "hm"));
        assert!(prompt.starts_with("Please address this review comment."));
    }

    #[test]
    fn explain_is_a_btw_question_with_a_capped_hunk() {
        let lines: Vec<String> = (0..80).map(|i| format!("+l{i}")).collect();
        let prompt = explain_prompt("x.rs", "@@ -1 +1 @@ ", &lines);
        assert!(prompt.starts_with("/btw Explain this change in x.rs (@@ -1 +1 @@)."));
        assert!(prompt.contains("+l59") && !prompt.contains("+l60"));
        assert!(prompt.ends_with("... 20 more lines"));
    }

    #[test]
    fn a_failed_send_restores_the_draft_in_front() {
        let mut drafts = Drafts::default();
        drafts.add(comment("a", "x.rs", "one"));
        let taken = drafts.take("a");
        drafts.add(comment("a", "y.rs", "later"));
        drafts.restore("a", taken);
        assert_eq!(drafts.of("a")[0].text, "one");
        assert_eq!(drafts.count_for("a", "y.rs", "@@ -1 +1 @@"), 1);
    }
}
