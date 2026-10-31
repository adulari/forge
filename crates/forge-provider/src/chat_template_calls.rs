//! Tool calls written in the chat template's own special tokens, recovered from assistant TEXT.
//!
//! Open-weight models trained with a tool-call template emit it verbatim when the serving stack's
//! parser does not claim the reply (a gateway that does not know the model, or no tools declared
//! on the request). Two families:
//!   - Kimi K2/K3: `<|tool_calls_section_begin|><|tool_call_begin|>functions.NAME:IDX
//!     <|tool_call_argument_begin|>{json}<|tool_call_end|>...<|tool_calls_section_end|>`
//!   - DeepSeek V3/R1: `<｜tool▁calls▁begin｜><｜tool▁call▁begin｜>function<｜tool▁sep｜>NAME
//!     ```json\n{json}\n```<｜tool▁call▁end｜><｜tool▁calls▁end｜>` (fullwidth bars, U+2581 spacers)
//!
//! Tokens are normalized to the ASCII `<|snake_case|>` spelling before matching, so a gateway
//! that flattened the bars or underscores is handled too. A missing `<|tool_call_end|>` is
//! tolerated when the next call, the section end or the end of text follows. Strict about
//! arguments: every call in a block must carry a JSON object, otherwise the block yields no calls
//! at all, so half a call is never executed (the caller asks the model to retry).
//! (DeepSeek V3.2+ DSML is the `dsml` module.)

use crate::tool_recovery::normalize_tool_name;
use forge_types::ToolCall;
use serde_json::Value;

const SECTION_BEGINS: [&str; 2] = ["<|tool_calls_section_begin|>", "<|tool_calls_begin|>"];
const SECTION_ENDS: [&str; 2] = ["<|tool_calls_section_end|>", "<|tool_calls_end|>"];
const CALL_BEGIN: &str = "<|tool_call_begin|>";
const CALL_END: &str = "<|tool_call_end|>";
const ARG_BEGIN: &str = "<|tool_call_argument_begin|>";
const SEP: &str = "<|tool_sep|>";
const EOS: &str = "<|end_of_sentence|>";

/// Rewrite every `<｜tool▁x｜>`-style special token in `s` to `<|tool_x|>`; text outside tokens is
/// untouched.
fn normalize_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        let bar_len = tail[1..]
            .chars()
            .next()
            .filter(|c| *c == '|' || *c == '｜')
            .map(char::len_utf8);
        let close = bar_len.and_then(|b| {
            tail[1 + b..]
                .find(['|', '｜'])
                .filter(|i| tail[1 + b..1 + b + i].len() < 48)
                .map(|i| (b, 1 + b + i))
        });
        match close {
            Some((b, end_bar)) if tail[end_bar..].chars().nth(1) == Some('>') => {
                let inner = &tail[1 + b..end_bar];
                let bar_end = tail[end_bar..].chars().next().map_or(1, char::len_utf8);
                out.push_str("<|");
                out.push_str(&inner.replace('▁', "_"));
                out.push_str("|>");
                rest = &tail[end_bar + bar_end + 1..];
            }
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether `pos` in `text` sits inside a fenced block or an inline code span, where a mention of
/// a token is documentation, not a call.
pub(crate) fn in_code(text: &str, pos: usize) -> bool {
    let before = &text[..pos];
    if before.matches("```").count() % 2 == 1 {
        return true;
    }
    let line = before.rsplit('\n').next().unwrap_or(before);
    line.replace("```", "").matches('`').count() % 2 == 1
}

/// Byte offset of the first real block opener: a complete section or call token outside code.
fn first_live_opener(text: &str) -> Option<usize> {
    SECTION_BEGINS
        .iter()
        .chain(std::iter::once(&CALL_BEGIN))
        .flat_map(|tok| text.match_indices(tok).map(|(i, _)| i))
        .filter(|i| !in_code(text, *i))
        .min()
}

/// Whether `content` carries a tool-call block in these templates' tokens: a complete opener
/// outside code, followed by an argument or separator token. A bare mention of the token names in
/// prose or code is an ordinary answer.
pub(crate) fn has_markup(content: &str) -> bool {
    if !["tool_call", "tool▁call", "tool_calls", "tool▁calls"]
        .iter()
        .any(|m| content.contains(m))
    {
        return false;
    }
    let text = normalize_tokens(content);
    first_live_opener(&text).is_some_and(|at| {
        let tail = &text[at..];
        SECTION_BEGINS.iter().any(|b| tail.starts_with(b))
            || tail.contains(ARG_BEGIN)
            || tail.contains(SEP)
    })
}

/// The calls in `content` and the text with the whole block removed. `None` when there is no
/// block, or it is malformed (cut off, bad JSON, unnamed call).
pub(crate) fn recover(content: &str) -> Option<(Vec<ToolCall>, String)> {
    if !has_markup(content) {
        return None;
    }
    let text = normalize_tokens(content);
    let first = first_live_opener(&text)?;
    let before = text[..first].trim_end().to_string();
    let mut tail = &text[first..];
    for b in SECTION_BEGINS {
        tail = tail.strip_prefix(b).unwrap_or(tail);
    }
    let end = SECTION_ENDS
        .iter()
        .find_map(|e| tail.find(e).map(|i| (i, e.len())));
    let (block, after) = match end {
        Some((i, len)) => (&tail[..i], text_after(tail, i + len)),
        None => (tail, ""),
    };
    let block = block.trim_end_matches(EOS);
    let mut calls = Vec::new();
    for raw in block.split(CALL_BEGIN).skip(1) {
        let body = raw.split(CALL_END).next().unwrap_or(raw).trim();
        calls.push(parse_call(body, calls.len())?);
    }
    if calls.is_empty() {
        return None;
    }
    let shown = [before.as_str(), after.trim()]
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n");
    Some((calls, shown))
}

fn text_after(s: &str, at: usize) -> &str {
    s.get(at..).unwrap_or("").trim_end_matches(EOS)
}

fn parse_call(body: &str, idx: usize) -> Option<ToolCall> {
    let (name, args) = if let Some((head, args)) = body.split_once(ARG_BEGIN) {
        (head.trim(), args)
    } else if let Some((_, rest)) = body.split_once(SEP) {
        let (name, args) = rest.split_once('\n').unwrap_or((rest, ""));
        (name.trim(), args)
    } else {
        return None;
    };
    // Kimi writes `functions.NAME:IDX`.
    let name = name.strip_prefix("functions.").unwrap_or(name);
    let name = name
        .rsplit_once(':')
        .filter(|(_, i)| i.chars().all(|c| c.is_ascii_digit()))
        .map_or(name, |(n, _)| n);
    if name.is_empty() {
        return None;
    }
    let args = args.trim();
    let args = args
        .strip_prefix("```json")
        .or_else(|| args.strip_prefix("```"))
        .map_or(args, |a| a.trim_end().trim_end_matches("```"))
        .trim();
    let args = if args.is_empty() { "{}" } else { args };
    let value: Value = serde_json::from_str(args).ok().filter(Value::is_object)?;
    Some(ToolCall {
        id: format!("recovered_{idx}"),
        name: normalize_tool_name(name),
        args: value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured shape: Kimi K3 answering through a gateway that did not claim the tool parser.
    const KIMI: &str = "I'll read it.<|tool_calls_section_begin|><|tool_call_begin|>functions.read_file:0<|tool_call_argument_begin|>{\"path\": \"src/a.rs\"}<|tool_call_end|><|tool_call_begin|>functions.shell:1<|tool_call_argument_begin|>{\"command\": \"ls\"}<|tool_call_end|><|tool_calls_section_end|>";

    // Captured shape: DeepSeek V3 template, fullwidth bars and U+2581.
    const DEEPSEEK: &str = "Checking.<｜tool▁calls▁begin｜><｜tool▁call▁begin｜>function<｜tool▁sep｜>read_file\n```json\n{\"path\": \"/etc/hostname\"}\n```<｜tool▁call▁end｜><｜tool▁calls▁end｜><｜end▁of▁sentence｜>";

    #[test]
    fn kimi_section_yields_every_call_and_the_prose() {
        let (calls, shown) = recover(KIMI).expect("calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].args["path"], "src/a.rs");
        assert_eq!(calls[1].name, "shell");
        assert_eq!(shown, "I'll read it.");
    }

    #[test]
    fn deepseek_v3_fullwidth_template_is_recovered() {
        let (calls, shown) = recover(DEEPSEEK).expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].args["path"], "/etc/hostname");
        assert_eq!(shown, "Checking.");
    }

    #[test]
    fn a_missing_call_end_is_tolerated() {
        let s = "<|tool_calls_section_begin|><|tool_call_begin|>functions.shell:0<|tool_call_argument_begin|>{\"command\":\"pwd\"}<|tool_calls_section_end|>";
        let (calls, _) = recover(s).expect("calls");
        assert_eq!(calls[0].args["command"], "pwd");
    }

    #[test]
    fn malformed_or_cut_off_blocks_run_nothing() {
        let bad_json = "<|tool_calls_section_begin|><|tool_call_begin|>functions.shell:0<|tool_call_argument_begin|>{\"command\":<|tool_call_end|><|tool_calls_section_end|>";
        assert!(recover(bad_json).is_none());
        assert!(
            recover("<|tool_calls_section_begin|><|tool_call_begin|>functions.shell:0").is_none()
        );
        assert!(recover("plain answer mentioning tool_call_begin in prose").is_none());
    }

    // Regression (review finding 15): prose and code that merely name the tokens are answers.
    #[test]
    fn mentioning_the_tokens_is_not_markup() {
        for text in [
            "`DsmlStreamGuard` now handles `tool_call_begin` and returns `Vec<ToolCall>`",
            "Kimi wraps calls in `<|tool_call_begin|>`...`<|tool_call_end|>` and fits in Vec<u8>.",
            "The token <|tool_call_begin|> opens a call; see Vec<T>.",
            "```\n<|tool_calls_section_begin|><|tool_call_begin|>functions.x:0<|tool_call_argument_begin|>{}<|tool_call_end|>\n```",
        ] {
            assert!(!has_markup(text), "{text}");
            assert!(recover(text).is_none(), "{text}");
        }
        assert!(has_markup(KIMI) && has_markup(DEEPSEEK));
    }

    #[test]
    fn flattened_ascii_deepseek_tokens_work() {
        let s = "<|tool▁calls▁begin|><|tool▁call▁begin|>function<|tool▁sep|>shell\n```json\n{\"command\":\"ls\"}\n```<|tool▁call▁end|><|tool▁calls▁end|>";
        assert_eq!(recover(s).expect("calls").0[0].name, "shell");
    }
}
