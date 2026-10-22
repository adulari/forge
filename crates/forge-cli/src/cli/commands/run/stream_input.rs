//! `--input-format stream-json`: one JSON message per stdin line, Claude Code's SDK input shape.
//!
//! A `user` message is a turn of the same session; its content is a string or a list of blocks
//! whose `text` blocks are joined. A `control_request` gets a `control_response`: `initialize`
//! succeeds (there is nothing to negotiate), anything else is reported as unsupported so an SDK
//! host fails fast instead of waiting forever. Other message types are ignored.

use serde_json::{json, Value};

#[derive(Debug, PartialEq)]
pub(crate) enum InputLine {
    Prompt(String),
    /// The `control_response` line to write back.
    Control(Value),
    Ignore,
}

pub(crate) fn parse_input_line(line: &str) -> Result<InputLine, String> {
    if line.trim().is_empty() {
        return Ok(InputLine::Ignore);
    }
    let value: Value =
        serde_json::from_str(line).map_err(|e| format!("not a JSON message ({e}): {line}"))?;
    match value.get("type").and_then(Value::as_str) {
        Some("user") => {
            let content = value
                .get("message")
                .and_then(|m| m.get("content"))
                .ok_or("a user message without `message.content`")?;
            let text = content_text(content);
            if text.trim().is_empty() {
                return Err("a user message without text".to_string());
            }
            Ok(InputLine::Prompt(text))
        }
        Some("control_request") => {
            let request_id = value.get("request_id").cloned().unwrap_or(Value::Null);
            let subtype = value
                .get("request")
                .and_then(|r| r.get("subtype"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let response = if subtype == "initialize" {
                json!({ "subtype": "success", "request_id": request_id, "response": {} })
            } else {
                json!({ "subtype": "error", "request_id": request_id,
                        "error": format!("control request `{subtype}` is not supported by forge") })
            };
            Ok(InputLine::Control(
                json!({ "type": "control_response", "response": response }),
            ))
        }
        _ => Ok(InputLine::Ignore),
    }
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_messages_become_prompts_from_strings_or_text_blocks() {
        assert_eq!(
            parse_input_line(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#),
            Ok(InputLine::Prompt("hi".into()))
        );
        let blocks = r#"{"type":"user","message":{"role":"user","content":[
            {"type":"text","text":"a"},{"type":"image","source":{}},{"type":"text","text":"b"}]}}"#;
        assert_eq!(
            parse_input_line(&blocks.replace('\n', "")),
            Ok(InputLine::Prompt("a\nb".into()))
        );
    }

    #[test]
    fn control_requests_are_answered_and_other_lines_ignored() {
        let init = parse_input_line(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"initialize"}}"#,
        )
        .unwrap();
        let InputLine::Control(reply) = init else {
            panic!("{init:?}")
        };
        assert_eq!(reply["response"]["subtype"], "success");
        assert_eq!(reply["response"]["request_id"], "r1");
        let other = parse_input_line(
            r#"{"type":"control_request","request_id":"r2","request":{"subtype":"interrupt"}}"#,
        )
        .unwrap();
        assert!(matches!(other, InputLine::Control(v) if v["response"]["subtype"] == "error"));
        assert_eq!(
            parse_input_line(r#"{"type":"keep_alive"}"#),
            Ok(InputLine::Ignore)
        );
        assert_eq!(parse_input_line("  "), Ok(InputLine::Ignore));
        assert!(parse_input_line("not json").is_err());
        assert!(parse_input_line(r#"{"type":"user","message":{"content":[]}}"#).is_err());
    }
}
