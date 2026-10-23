//! The two meta-tools of the lean tool surface (docs/features/deferred-tools.md), and what a
//! `tool_call` really calls.
//!
//! `tool_search` only looks tools up. `tool_call{name, arguments}` runs one by name, so every place
//! that judges, shows or remembers a call has to look at the tool inside it, not at the wrapper.

use serde_json::{Map, Value};

pub const TOOL_SEARCH: &str = "tool_search";
pub const TOOL_CALL: &str = "tool_call";

/// The tool a `tool_call`'s arguments name and the arguments it passes on. Arguments may arrive as
/// an object, as an escaped JSON string (weak models), or under `args`/`input`. `Err` is the
/// model-facing reason when the call names nothing runnable.
pub fn inner_call(args: &Value) -> Result<(String, Value), String> {
    let inner = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if inner.is_empty() || inner == TOOL_CALL || inner == TOOL_SEARCH {
        return Err(
            "error: tool_call needs the exact name of a tool found with tool_search".to_string(),
        );
    }
    let inner_args = match ["arguments", "args", "input"]
        .iter()
        .find_map(|k| args.get(*k))
    {
        Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or(Value::String(raw.clone())),
        Some(Value::Null) | None => Value::Object(Map::new()),
        Some(other) => other.clone(),
    };
    Ok((inner.to_string(), inner_args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_inner_call_is_the_name_and_the_arguments_it_carries() {
        let (name, args) =
            inner_call(&json!({"name": "shell", "arguments": {"command": "ls"}})).unwrap();
        assert_eq!((name.as_str(), &args), ("shell", &json!({"command": "ls"})));
    }

    #[test]
    fn arguments_may_be_an_escaped_string_or_sit_under_another_key() {
        let (_, args) =
            inner_call(&json!({"name": "web_fetch", "arguments": "{\"url\":\"https://x\"}"}))
                .unwrap();
        assert_eq!(args["url"], "https://x");
        let (_, args) = inner_call(&json!({"name": "device", "args": {"action": "ui"}})).unwrap();
        assert_eq!(args["action"], "ui");
        let (_, args) = inner_call(&json!({"name": "device", "input": {"action": "ui"}})).unwrap();
        assert_eq!(args["action"], "ui");
    }

    #[test]
    fn missing_arguments_are_an_empty_object() {
        let (_, args) = inner_call(&json!({"name": "shell_job"})).unwrap();
        assert_eq!(args, json!({}));
        let (_, args) = inner_call(&json!({"name": "shell_job", "arguments": null})).unwrap();
        assert_eq!(args, json!({}));
    }

    #[test]
    fn a_name_that_runs_nothing_is_refused() {
        for args in [
            json!({}),
            json!({"name": ""}),
            json!({"name": TOOL_CALL}),
            json!({"name": TOOL_SEARCH}),
        ] {
            assert!(inner_call(&args).is_err(), "{args}");
        }
    }
}
