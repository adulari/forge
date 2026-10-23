//! DeepSeek DSML tool-call markup, recovered from assistant TEXT.
//!
//! DeepSeek's chat template writes a tool call as text and relies on the serving stack's tool
//! parser to turn it into a structured call. When the parser does not claim the reply (no tools
//! were declared on the request, a gateway that does not know the model, a block the model got
//! slightly wrong) the markup arrives as plain `content` and nothing runs.
//!
//! Dialects seen or documented (`｜` is U+FF5C; a gateway may flatten it to ASCII `|`, and the
//! template's own namespace token is `｜DSML｜`, which the model sometimes doubles):
//!   - V3.2: `<｜DSML｜function_calls>` wrapping `<｜DSML｜invoke name="T">` blocks.
//!   - V4:   the same with `tool_calls` as the wrapper.
//!   - V4.1: a space after the namespace and the wrapper shortened to `calls`:
//!     `<｜DSML｜ calls>`, `<｜DSML｜ invoke name="T">`, `<｜DSML｜ parameter name="p" string="true">`.
//!
//! Every parameter is `<ns parameter name="p" string="true|false">raw text</ns parameter>`:
//! `string="true"` is literal text, `string="false"` is a JSON value. Values are raw (not
//! XML-escaped), so a closing tag is only ever the matching DSML tag, never any `<`.
//!
//! The parser is lenient about spelling (bars, whitespace, wrapper name) and strict about
//! structure: a block runs only if every tag in it is closed and every value is well-typed. A
//! block cut off mid-call, with a bad JSON value or with stray text inside, yields no calls at all,
//! so half a call is never executed; the caller then asks the model to retry.

use crate::tool_recovery::{coerce, normalize_tool_name};
use forge_types::ToolCall;
use serde_json::{Map, Value};

pub(crate) const MARKER: &str = "DSML";
const EOS: &str = "<｜end▁of▁sentence｜>";

/// Whether `c` is a vertical bar of the template's `｜DSML｜` namespace.
pub(crate) fn is_dsml_bar(c: char) -> bool {
    c == '\u{ff5c}' || c == '|'
}

struct Tag<'a> {
    end: usize,
    closing: bool,
    name: &'a str,
    attrs: &'a str,
}

/// The DSML tag that starts at byte `start` of `s`, if one does.
fn tag_at(s: &str, start: usize) -> Option<Tag<'_>> {
    let body = s.get(start..)?.strip_prefix('<')?;
    let (closing, body) = match body.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, body),
    };
    let after_bars = body.trim_start_matches(is_dsml_bar);
    if after_bars.len() == body.len() {
        return None;
    }
    let after_marker = after_bars.strip_prefix(MARKER)?;
    let after_closing_bars = after_marker.trim_start_matches(is_dsml_bar);
    if after_closing_bars.len() == after_marker.len() {
        return None;
    }
    let named = after_closing_bars.trim_start();
    let name_len = named
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(named.len());
    if name_len == 0 {
        return None;
    }
    let (name, tail) = named.split_at(name_len);
    let mut quote = None;
    let mut gt = None;
    for (i, c) in tail.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => {
                gt = Some(i);
                break;
            }
            _ => {}
        }
    }
    let gt = gt?;
    Some(Tag {
        end: s.len() - tail.len() + gt + 1,
        closing,
        name,
        attrs: &tail[..gt],
    })
}

/// `key="value"` pairs of an opening tag; `None` when the attribute text is not well-formed.
fn parse_attrs(attrs: &str) -> Option<Vec<(&str, &str)>> {
    let mut out = Vec::new();
    let mut rest = attrs.trim();
    while !rest.is_empty() {
        let (key, after_eq) = rest.split_once('=')?;
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            return None;
        }
        let after_eq = after_eq.trim_start();
        let quote = after_eq
            .chars()
            .next()
            .filter(|q| matches!(q, '"' | '\''))?;
        let value_and_rest = &after_eq[1..];
        let close = value_and_rest.find(quote)?;
        out.push((key, &value_and_rest[..close]));
        rest = value_and_rest[close + 1..].trim_start();
    }
    Some(out)
}

fn attr<'a>(attrs: &[(&str, &'a str)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn is_wrapper(name: &str) -> bool {
    ["tool_calls", "function_calls", "calls"]
        .iter()
        .any(|w| name.eq_ignore_ascii_case(w))
}

/// The closing `</ns name>` tag at or after `from`: `(start, end)`.
fn find_close(s: &str, from: usize, name: &str) -> Option<(usize, usize)> {
    let mut at = from;
    while let Some(rel) = s.get(at..)?.find("</") {
        let start = at + rel;
        if let Some(tag) =
            tag_at(s, start).filter(|t| t.closing && t.name.eq_ignore_ascii_case(name))
        {
            return Some((start, tag.end));
        }
        at = start + 2;
    }
    None
}

fn skip_whitespace(s: &str, pos: usize) -> usize {
    s.len() - s[pos..].trim_start().len()
}

/// One `<ns invoke name="T">…</ns invoke>` starting at `open`: the call and where it ends.
fn parse_invoke(s: &str, open: &Tag<'_>) -> Option<((String, Value), usize)> {
    let attrs = parse_attrs(open.attrs)?;
    let name = attr(&attrs, "name")?.trim();
    if name.is_empty() {
        return None;
    }
    let mut args = Map::new();
    let mut pos = open.end;
    loop {
        pos = skip_whitespace(s, pos);
        let tag = tag_at(s, pos)?;
        if tag.closing {
            let done = tag.name.eq_ignore_ascii_case("invoke");
            return done.then(|| ((normalize_tool_name(name), Value::Object(args)), tag.end));
        }
        if !tag.name.eq_ignore_ascii_case("parameter") {
            return None;
        }
        let attrs = parse_attrs(tag.attrs)?;
        let key = attr(&attrs, "name")?.trim();
        if key.is_empty() {
            return None;
        }
        let (value_end, close_end) = find_close(s, tag.end, "parameter")?;
        let raw = &s[tag.end..value_end];
        let value = match attr(&attrs, "string") {
            Some("true") => Value::String(raw.to_string()),
            Some("false") => serde_json::from_str(raw.trim()).ok()?,
            Some(_) => return None,
            None => coerce(raw.trim()),
        };
        if args.insert(key.to_string(), value).is_some() {
            return None;
        }
        pos = close_end;
    }
}

/// One block starting at its first tag (a wrapper, or a bare invoke): its calls and where it ends.
/// `None` when the block is partial or malformed.
fn parse_block(s: &str, start: usize) -> Option<(Vec<(String, Value)>, usize)> {
    let mut calls = Vec::new();
    let mut wrapped = false;
    let mut pos = start;
    let mut end = start;
    loop {
        pos = skip_whitespace(s, pos);
        let Some(tag) = tag_at(s, pos) else {
            return (!wrapped && !calls.is_empty()).then_some((calls, end));
        };
        match (tag.closing, is_wrapper(tag.name)) {
            (true, true) => return (wrapped && !calls.is_empty()).then_some((calls, tag.end)),
            (false, true) if !wrapped => wrapped = true,
            (false, false) if tag.name.eq_ignore_ascii_case("invoke") => {
                let (call, call_end) = parse_invoke(s, &tag)?;
                calls.push(call);
                pos = call_end;
                end = call_end;
                continue;
            }
            _ => return None,
        }
        pos = tag.end;
        end = tag.end;
    }
}

/// Whether a DSML tag begins at `start`, even if the text ends before the tag does.
fn opener_at(s: &str, start: usize) -> bool {
    let Some(body) = s.get(start..).and_then(|rest| rest.strip_prefix('<')) else {
        return false;
    };
    let body = body.strip_prefix('/').unwrap_or(body);
    let after_bars = body.trim_start_matches(is_dsml_bar);
    after_bars.len() < body.len()
        && after_bars
            .strip_prefix(MARKER)
            .is_some_and(|rest| rest.starts_with(is_dsml_bar))
}

/// Start of the next DSML tag at or after `from` that is not inside a markdown code fence (a
/// documentation example is not a call).
fn next_live_tag(s: &str, from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(rel) = s.get(at..)?.find('<') {
        let start = at + rel;
        if opener_at(s, start) && s[..start].matches("```").count().is_multiple_of(2) {
            return Some(start);
        }
        at = start + 1;
    }
    None
}

/// Whether `content` holds DSML tool-call markup, complete or not.
pub(crate) fn has_markup(content: &str) -> bool {
    content.contains(MARKER) && next_live_tag(content, 0).is_some()
}

/// Recover DSML calls from `content`. `None` when there is no DSML markup. Otherwise the calls and
/// the visible text with every DSML block removed. A partial or malformed block yields no calls
/// (from the first broken block on, the rest is markup and is cut from the visible text too).
pub(crate) fn recover(content: &str) -> Option<(Vec<ToolCall>, String)> {
    let mut at = next_live_tag(content, 0)?;
    let mut visible = String::new();
    let mut found = Vec::new();
    let mut copied = 0;
    let mut broken = false;
    loop {
        visible.push_str(&content[copied..at]);
        match parse_block(content, at) {
            Some((calls, end)) => {
                found.extend(calls);
                copied = end;
            }
            None => {
                broken = true;
                copied = content.len();
                break;
            }
        }
        match next_live_tag(content, copied) {
            Some(next) => at = next,
            None => break,
        }
    }
    visible.push_str(&content[copied..]);
    let visible = visible.replace(EOS, "").trim().to_string();
    if broken {
        return Some((Vec::new(), visible));
    }
    let calls = found
        .into_iter()
        .enumerate()
        .map(|(idx, (name, args))| ToolCall {
            id: format!("recovered_{idx}"),
            name,
            args,
        })
        .collect();
    Some((calls, visible))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const V41: &str = "<｜｜DSML｜｜ calls>\n\
<｜｜DSML｜｜ invoke name=\"read_file\">\n\
<｜｜DSML｜｜ parameter name=\"path\" string=\"true\">src/lib.rs</｜｜DSML｜｜ parameter>\n\
</｜｜DSML｜｜ invoke>\n\
</｜｜DSML｜｜ calls>";

    fn calls(content: &str) -> Vec<ToolCall> {
        recover(content).expect("DSML markup").0
    }

    fn visible(content: &str) -> String {
        recover(content).expect("DSML markup").1
    }

    #[test]
    fn v41_dialect_with_a_space_after_the_namespace_and_calls_as_wrapper() {
        let found = calls(V41);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "read_file");
        assert_eq!(found[0].args, json!({"path": "src/lib.rs"}));
        assert_eq!(found[0].id, "recovered_0");
        assert_eq!(visible(V41), "");
    }

    #[test]
    fn every_documented_dialect_parses_to_the_same_call() {
        let block = |ns: &str, wrapper: &str, sp: &str| {
            format!(
                "<{ns}{sp}{wrapper}><{ns}{sp}invoke name=\"shell\">\
                 <{ns}{sp}parameter name=\"command\" string=\"true\">ls -la</{ns}{sp}parameter>\
                 </{ns}{sp}invoke></{ns}{sp}{wrapper}>"
            )
        };
        for (ns, wrapper, sp) in [
            ("｜DSML｜", "function_calls", ""),
            ("｜DSML｜", "tool_calls", ""),
            ("｜｜DSML｜｜", "tool_calls", ""),
            ("｜DSML｜", "calls", " "),
            ("｜｜DSML｜｜", "calls", " "),
            ("|DSML|", "tool_calls", ""),
            ("|DSML|", "calls", "  "),
        ] {
            let found = calls(&block(ns, wrapper, sp));
            assert_eq!(found.len(), 1, "{ns}{sp}{wrapper}");
            assert_eq!(found[0].args, json!({"command": "ls -la"}));
        }
    }

    #[test]
    fn typed_parameters_nested_json_and_literal_strings() {
        let text = "<｜DSML｜ calls><｜DSML｜ invoke name=\"edit\">\
            <｜DSML｜ parameter name=\"path\" string=\"true\">a.py</｜DSML｜ parameter>\
            <｜DSML｜ parameter name=\"edits\" string=\"false\">[{\"old\": \"a > b\", \"new\": \"{x: [1, 2]}\"}]</｜DSML｜ parameter>\
            <｜DSML｜ parameter name=\"line\" string=\"false\">42</｜DSML｜ parameter>\
            <｜DSML｜ parameter name=\"all\" string=\"false\">true</｜DSML｜ parameter>\
            <｜DSML｜ parameter name=\"code\" string=\"true\">42</｜DSML｜ parameter>\
            <｜DSML｜ parameter name=\"body\" string=\"true\">\n  <div>x</div>\n</parameter>\n</｜DSML｜ parameter>\
            </｜DSML｜ invoke></｜DSML｜ calls>";
        let found = calls(text);
        assert_eq!(
            found[0].args,
            json!({
                "path": "a.py",
                "edits": [{"old": "a > b", "new": "{x: [1, 2]}"}],
                "line": 42,
                "all": true,
                "code": "42",
                "body": "\n  <div>x</div>\n</parameter>\n",
            })
        );
    }

    #[test]
    fn several_invokes_keep_their_order_and_get_distinct_ids() {
        let text = "<｜DSML｜tool_calls>\
            <｜DSML｜invoke name=\"read_file\"><｜DSML｜parameter name=\"path\" string=\"true\">a</｜DSML｜parameter></｜DSML｜invoke>\
            <｜DSML｜invoke name=\"list_dir\"></｜DSML｜invoke>\
            <｜DSML｜invoke name=\"search\"><｜DSML｜parameter name=\"query\" string=\"true\">x</｜DSML｜parameter></｜DSML｜invoke>\
            </｜DSML｜tool_calls>";
        let found = calls(text);
        let names: Vec<_> = found.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["read_file", "list_dir", "search"]);
        let ids: Vec<_> = found.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["recovered_0", "recovered_1", "recovered_2"]);
        assert_eq!(found[1].args, json!({}));
    }

    #[test]
    fn unknown_and_namespaced_tool_names_are_kept_for_the_dispatcher_to_judge() {
        let text = "<｜DSML｜ calls>\
            <｜DSML｜ invoke name=\"no_such_tool\"></｜DSML｜ invoke>\
            <｜DSML｜ invoke name=\"math::add\"></｜DSML｜ invoke>\
            <｜DSML｜ invoke name=\"default_api:glob\"></｜DSML｜ invoke>\
            </｜DSML｜ calls>";
        let names: Vec<_> = calls(text).into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["no_such_tool", "math::add", "glob"]);
    }

    #[test]
    fn prose_before_and_after_survives_and_the_eos_token_does_not() {
        let text = format!("Reading it.\n\n{V41}\n\nThen I will explain.<｜end▁of▁sentence｜>");
        let (found, shown) = recover(&text).unwrap();
        assert_eq!(found.len(), 1);
        assert!(shown.starts_with("Reading it."), "{shown:?}");
        assert!(shown.ends_with("Then I will explain."), "{shown:?}");
        assert!(
            !shown.contains("DSML") && !shown.contains("end▁of"),
            "{shown:?}"
        );
    }

    #[test]
    fn two_blocks_in_one_reply_are_both_recovered() {
        let text = format!("{V41}\nand again\n{V41}");
        let (found, shown) = recover(&text).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].id, "recovered_1");
        assert_eq!(shown, "and again");
    }

    #[test]
    fn a_bare_invoke_without_a_wrapper_is_recovered() {
        let text = "ok <｜DSML｜ invoke name=\"glob\"><｜DSML｜ parameter name=\"pattern\" string=\"true\">*.rs</｜DSML｜ parameter></｜DSML｜ invoke> done";
        let (found, shown) = recover(text).unwrap();
        assert_eq!(found[0].args, json!({"pattern": "*.rs"}));
        assert_eq!(shown, "ok  done");
    }

    #[test]
    fn partial_and_malformed_blocks_run_nothing_and_leave_no_markup() {
        let cases = [
            ("cut off inside a value", "<｜DSML｜ calls><｜DSML｜ invoke name=\"read_file\"><｜DSML｜ parameter name=\"path\" string=\"true\">src/li"),
            ("cut off after the opening wrapper", "<｜DSML｜ calls>"),
            ("cut off inside a tag", "<｜DSML｜ calls><｜DSML｜ invoke name=\"read_fi"),
            ("invoke never closed", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"><｜DSML｜ parameter name=\"p\" string=\"true\">v</｜DSML｜ parameter></｜DSML｜ calls>"),
            ("wrapper never closed", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"></｜DSML｜ invoke>"),
            ("second invoke cut off", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"></｜DSML｜ invoke><｜DSML｜ invoke name=\"b\"><｜DSML｜ parameter name=\"p\" string=\"true\">v"),
            ("json value that is not json", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"><｜DSML｜ parameter name=\"n\" string=\"false\">[1, 2</｜DSML｜ parameter></｜DSML｜ invoke></｜DSML｜ calls>"),
            ("unknown string flag", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"><｜DSML｜ parameter name=\"n\" string=\"maybe\">x</｜DSML｜ parameter></｜DSML｜ invoke></｜DSML｜ calls>"),
            ("invoke without a name", "<｜DSML｜ calls><｜DSML｜ invoke><｜DSML｜ parameter name=\"p\" string=\"true\">v</｜DSML｜ parameter></｜DSML｜ invoke></｜DSML｜ calls>"),
            ("parameter without a name", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"><｜DSML｜ parameter string=\"true\">v</｜DSML｜ parameter></｜DSML｜ invoke></｜DSML｜ calls>"),
            ("duplicate parameter", "<｜DSML｜ calls><｜DSML｜ invoke name=\"a\"><｜DSML｜ parameter name=\"p\" string=\"true\">1</｜DSML｜ parameter><｜DSML｜ parameter name=\"p\" string=\"true\">2</｜DSML｜ parameter></｜DSML｜ invoke></｜DSML｜ calls>"),
            ("text inside the wrapper", "<｜DSML｜ calls>sure<｜DSML｜ invoke name=\"a\"></｜DSML｜ invoke></｜DSML｜ calls>"),
            ("parameter outside an invoke", "<｜DSML｜ calls><｜DSML｜ parameter name=\"p\" string=\"true\">v</｜DSML｜ parameter></｜DSML｜ calls>"),
            ("empty wrapper", "<｜DSML｜ calls></｜DSML｜ calls>"),
            ("stray closing tag", "</｜DSML｜ calls>"),
        ];
        for (label, body) in cases {
            let (found, shown) = recover(&format!("Checking.\n{body}")).unwrap();
            assert!(found.is_empty(), "{label}: ran {found:?}");
            assert_eq!(shown, "Checking.", "{label}");
        }
    }

    #[test]
    fn one_broken_block_cancels_the_whole_reply() {
        let text = format!("{V41}\n<｜DSML｜ calls><｜DSML｜ invoke name=\"b\">");
        let (found, shown) = recover(&text).unwrap();
        assert!(found.is_empty());
        assert_eq!(shown, "");
    }

    #[test]
    fn a_documentation_example_inside_a_code_fence_is_not_a_call() {
        let text = format!("The template looks like this:\n```\n{V41}\n```\nThat is all.");
        assert!(recover(&text).is_none());
        assert!(!has_markup(&text));
        let after_fence = format!("```\nx\n```\n{V41}");
        assert_eq!(recover(&after_fence).unwrap().0.len(), 1);
    }

    #[test]
    fn ordinary_text_is_not_markup() {
        for text in [
            "",
            "DSML is DeepSeek's markup",
            "a < b and <div>x</div>",
            "<｜DSML only after a bar>",
            "<DSML｜ calls>",
            "<｜｜｜ calls>",
        ] {
            assert!(!has_markup(text), "{text:?}");
            assert!(recover(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn has_markup_sees_partial_blocks() {
        assert!(has_markup("hi <｜DSML｜ calls>"));
        assert!(has_markup("hi <｜｜DSML｜｜ invoke name=\"x\""));
        assert!(has_markup("hi <｜DSML｜"));
        assert!(has_markup(V41));
    }
}
