//! The episode index a compaction summary carries: which message seq ranges were folded, each with
//! one topic line, so the model knows what the `recall` tool can reach.
//!
//! Built by the harness, not the summarizer, so it costs no model output and cannot be dropped or
//! paraphrased: each folded range is cut at the user's own prompts, and the topic is the first line
//! of the prompt that opened the episode. The list stays within [`MAX_EPISODE_LINES`] by merging
//! the neighbours that cover the fewest messages, and a later compaction reads the previous list
//! back out of the summary it absorbs, so the index survives summary-of-summaries.

use forge_store::FoldPreviewRow;

use crate::compaction_summary::SUMMARY_PREFIX;

/// The heading of the section, which `split_episodes` looks for.
pub(crate) const EPISODES_HEADING: &str = "## Earlier episodes (message seq ranges)";
const EPISODES_INTRO: &str = "These messages were folded out of your context. Each is still stored: use the `recall` tool to search them or read a seq range, before you ask the user or drop work whose detail you cannot place.";
/// Lines in the list, merged down to this many.
pub(crate) const MAX_EPISODE_LINES: usize = 12;
const TOPIC_CHARS: usize = 100;
const MERGED_TOPIC_CHARS: usize = 130;

/// One folded seq range and what it was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Episode {
    pub from: i64,
    pub to: i64,
    /// Empty for rows that continue an episode opened before the range (no prompt of their own).
    pub topic: String,
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    clip(&collapsed, TOPIC_CHARS)
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text.to_string(),
    }
}

fn is_harness_prompt(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("[stop hook] ")
        || text.starts_with("[background job ")
        || text.starts_with("Auto-fix: the following checks failed")
        || text.starts_with(SUMMARY_PREFIX)
}

/// Cut the rows a compaction is about to fold into episodes, one per real user prompt. Rows before
/// the first prompt become a topic-less leading episode.
pub(crate) fn episodes_from_fold(rows: &[FoldPreviewRow]) -> Vec<Episode> {
    let mut out: Vec<Episode> = Vec::new();
    for row in rows {
        let opens = row.role == "user"
            && !row.nudge
            && !row.head.trim().is_empty()
            && !is_harness_prompt(&row.head);
        if opens {
            out.push(Episode {
                from: row.seq,
                to: row.seq,
                topic: first_line(&row.head),
            });
        } else if let Some(last) = out.last_mut() {
            last.to = row.seq;
        } else {
            out.push(Episode {
                from: row.seq,
                to: row.seq,
                topic: String::new(),
            });
        }
    }
    out
}

/// The previous index (continued with `new`), merged down to the line limit, and rendered as the
/// summary section. Empty when there is nothing to list.
pub(crate) fn render_episodes(previous: &[Episode], new: &[Episode]) -> String {
    let mut all: Vec<Episode> = previous.to_vec();
    for (i, episode) in new.iter().enumerate() {
        match (i, all.last_mut()) {
            // Rows that continue the last episode of the earlier range.
            (0, Some(last)) if episode.topic.is_empty() => last.to = episode.to,
            _ => all.push(episode.clone()),
        }
    }
    if all.is_empty() {
        return String::new();
    }
    while all.len() > MAX_EPISODE_LINES {
        let at = (0..all.len() - 1)
            .min_by_key(|&i| all[i + 1].to - all[i].from)
            .unwrap_or(0);
        let next = all.remove(at + 1);
        let this = &mut all[at];
        this.to = next.to;
        this.topic = match (this.topic.is_empty(), next.topic.is_empty()) {
            (_, true) => this.topic.clone(),
            (true, false) => next.topic,
            (false, false) => clip(
                &format!("{} / {}", this.topic, next.topic),
                MERGED_TOPIC_CHARS,
            ),
        };
    }
    let mut out = format!("{EPISODES_HEADING}\n{EPISODES_INTRO}\n");
    for episode in &all {
        let topic = if episode.topic.is_empty() {
            "(continued work)"
        } else {
            &episode.topic
        };
        out.push_str(&format!("- seq {}-{}: {topic}\n", episode.from, episode.to));
    }
    out.trim_end().to_string()
}

fn parse_line(line: &str) -> Option<Episode> {
    let rest = line.strip_prefix("- seq ")?;
    let (range, topic) = rest.split_once(": ")?;
    let (from, to) = range.split_once('-')?;
    let topic = topic.trim();
    Some(Episode {
        from: from.parse().ok()?,
        to: to.parse().ok()?,
        topic: if topic == "(continued work)" {
            String::new()
        } else {
            topic.to_string()
        },
    })
}

/// A summary body split into what the model wrote and the episode list a previous compaction
/// appended to it. The body is returned without the section.
pub(crate) fn split_episodes(body: &str) -> (&str, Vec<Episode>) {
    let Some(start) = body.find(EPISODES_HEADING) else {
        return (body, Vec::new());
    };
    let episodes = body[start + EPISODES_HEADING.len()..]
        .lines()
        .filter_map(parse_line)
        .collect();
    (body[..start].trim_end(), episodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seq: i64, role: &str, head: &str) -> FoldPreviewRow {
        FoldPreviewRow {
            seq,
            role: role.into(),
            nudge: false,
            head: head.into(),
        }
    }

    #[test]
    fn episodes_open_at_real_user_prompts_and_skip_nudges() {
        let mut nudge = row(4, "user", "Continue");
        nudge.nudge = true;
        let rows = [
            row(0, "user", "\n  add the retry policy\nmore detail"),
            row(1, "assistant", ""),
            row(2, "tool", ""),
            row(3, "user", "now the parser"),
            nudge,
            row(5, "assistant", ""),
        ];
        let episodes = episodes_from_fold(&rows);
        assert_eq!(
            episodes,
            [
                Episode {
                    from: 0,
                    to: 2,
                    topic: "add the retry policy".into()
                },
                Episode {
                    from: 3,
                    to: 5,
                    topic: "now the parser".into()
                },
            ]
        );
    }

    #[test]
    fn rows_before_the_first_prompt_continue_the_previous_episode() {
        let first = [Episode {
            from: 0,
            to: 9,
            topic: "build x".into(),
        }];
        let new = episodes_from_fold(&[row(10, "assistant", ""), row(11, "user", "then y")]);
        let text = render_episodes(&first, &new);
        assert!(text.contains("- seq 0-10: build x"), "{text}");
        assert!(text.contains("- seq 11-11: then y"), "{text}");
    }

    #[test]
    fn the_list_is_merged_to_the_line_limit_and_round_trips() {
        let new: Vec<Episode> = (0..40)
            .map(|i| Episode {
                from: i * 10,
                to: i * 10 + 9,
                topic: format!("topic {i}"),
            })
            .collect();
        let text = render_episodes(&[], &new);
        let lines = text.lines().filter(|l| l.starts_with("- seq ")).count();
        assert_eq!(lines, MAX_EPISODE_LINES);
        let (body, parsed) = split_episodes(&text);
        assert!(body.is_empty());
        assert_eq!(parsed.len(), MAX_EPISODE_LINES);
        assert_eq!(parsed.first().unwrap().from, 0);
        assert_eq!(
            parsed.last().unwrap().to,
            399,
            "the whole range stays covered"
        );
        assert!(parsed.windows(2).all(|w| w[0].to < w[1].from));
    }

    #[test]
    fn the_index_is_carried_forward_across_compactions() {
        let first = render_episodes(
            &[],
            &episodes_from_fold(&[row(0, "user", "first job"), row(1, "assistant", "")]),
        );
        let summary = format!("## 1. Primary request\nstuff\n\n{first}");
        let (body, previous) = split_episodes(&summary);
        assert_eq!(body, "## 1. Primary request\nstuff");
        let second = render_episodes(
            &previous,
            &episodes_from_fold(&[row(8, "user", "second job")]),
        );
        assert!(
            second.contains("- seq 0-1: first job") && second.contains("- seq 8-8: second job")
        );
    }

    #[test]
    fn a_summary_without_a_section_has_no_episodes() {
        assert_eq!(split_episodes("just a summary"), ("just a summary", vec![]));
        assert_eq!(render_episodes(&[], &[]), "");
    }
}
