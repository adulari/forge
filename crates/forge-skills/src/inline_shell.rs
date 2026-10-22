//! Claude Code's dynamic context injection: `` !`cmd` `` and fenced ` ```! ` blocks in a skill or
//! command body are shell commands whose output replaces them before the model sees the text.
//!
//! This module only finds and replaces them; running them is the caller's job, because whether a
//! command may run at all depends on the session (project trust, permission rules, the safety
//! floor). The inline form counts only at the start of a line or right after whitespace, so
//! ``KEY=!`cmd` `` and prose like `` `panic!` `` stay literal. Output is inserted once and never
//! scanned again.

use std::ops::Range;

/// One command found in a template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellSnippet {
    /// The template text the output replaces, for error messages (`` !`gh pr diff` ``).
    pub pattern: String,
    pub command: String,
    range: Range<usize>,
}

/// Every injected command in `text`, in order.
pub fn find(text: &str) -> Vec<ShellSnippet> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let start = offset;
        offset += line.len();
        if line.trim() == "```!" {
            let mut body = String::new();
            let mut end = offset;
            let mut closed = false;
            while let Some(next) = lines.peek() {
                let next = *next;
                lines.next();
                end += next.len();
                if next.trim() == "```" {
                    closed = true;
                    break;
                }
                body.push_str(next);
            }
            if closed {
                let fence = &text[start..end];
                let fence = fence.strip_suffix('\n').unwrap_or(fence);
                out.push(ShellSnippet {
                    pattern: fence.to_string(),
                    command: body.trim_end().to_string(),
                    range: start..start + fence.len(),
                });
                offset = end;
                continue;
            }
            // An unclosed fence is plain text: rescan its lines as ordinary lines.
            lines = text[offset..].split_inclusive('\n').peekable();
        }
        inline_snippets(line, start, &mut out);
    }
    out
}

fn inline_snippets(line: &str, line_start: usize, out: &mut Vec<ShellSnippet>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let at_boundary = i == 0 || bytes[i - 1].is_ascii_whitespace();
        if bytes[i] == b'!' && bytes[i + 1] == b'`' && at_boundary {
            if let Some(close) = line[i + 2..].find('`') {
                let command = &line[i + 2..i + 2 + close];
                let end = i + 2 + close + 1;
                if !command.trim().is_empty() && !command.contains('\n') {
                    out.push(ShellSnippet {
                        pattern: line[i..end].to_string(),
                        command: command.trim().to_string(),
                        range: line_start + i..line_start + end,
                    });
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// `text` with each snippet replaced by its output (`outputs[i]` for `snippets[i]`, trailing
/// newlines trimmed).
pub fn render(text: &str, snippets: &[ShellSnippet], outputs: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (snippet, output) in snippets.iter().zip(outputs) {
        out.push_str(&text[cursor..snippet.range.start]);
        out.push_str(output.trim_end_matches(['\n', '\r']));
        cursor = snippet.range.end;
    }
    out.push_str(&text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_commands_count_only_at_a_word_start() {
        let text = "- diff: !`gh pr diff`\n!`git status --short` and KEY=!`nope` or `panic!` too";
        let found = find(text);
        let commands: Vec<&str> = found.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(commands, ["gh pr diff", "git status --short"]);
        assert_eq!(found[0].pattern, "!`gh pr diff`");
        let rendered = render(text, &found, &["D\n".into(), "S".into()]);
        assert_eq!(rendered, "- diff: D\nS and KEY=!`nope` or `panic!` too");
    }

    #[test]
    fn fenced_blocks_run_as_one_script() {
        let text = "## Env\n```!\nnode --version\ngit status --short\n```\nafter";
        let found = find(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "node --version\ngit status --short");
        assert_eq!(
            render(text, &found, &["v22\nclean".into()]),
            "## Env\nv22\nclean\nafter"
        );
    }

    #[test]
    fn unclosed_fence_and_empty_commands_stay_literal() {
        assert!(find("```!\nnever closed").is_empty());
        assert!(find("empty !`` here").is_empty());
        let found = find("```!\nunclosed !`date`");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command, "date");
    }

    #[test]
    fn text_without_commands_renders_unchanged() {
        let text = "plain `code` and ! bang";
        assert!(find(text).is_empty());
        assert_eq!(render(text, &[], &[]), text);
    }
}
