//! Models that fill in every property of a tool.
//!
//! A model asked to call a tool with many optional properties often emits all of them, each with
//! the value that means "not given": `commands: []`, `paths: []`, `path: ""`, `file_pattern: ""`,
//! `timeout_secs: 0`, `null`. (OpenAI's strict function-call mode forces exactly this: every
//! property present, so an absent one has to be spelled with a placeholder.) A tool that tells
//! "absent" from "present" with `args.get(key).is_some()` then sees a contradiction, such as
//! `shell` with both `command` and `commands`.
//!
//! [`normalize_args`] removes those placeholders before the call reaches the permission rules, the
//! hooks and the tool, using the tool's own schema: only an optional property (one the schema does
//! not list in `required`) whose value is the empty value of its declared type is dropped, and
//! never a boolean (`false` is a real answer). A property whose schema says `"minLength": 0`
//! accepts the empty string as a value and keeps it. Required properties, unknown properties and
//! every non-empty value pass through untouched.

use std::collections::HashSet;

use serde_json::{Map, Value};

/// `args` without the placeholder values a strict-mode model fills in for absent optional
/// properties. Nested objects, and the objects inside arrays, are normalised against their own
/// `properties`/`items` schema. Anything that is not an object comes back unchanged.
pub fn normalize_args(schema: &Value, args: &Value) -> Value {
    match args {
        Value::Object(map) => Value::Object(normalize_object(schema, map)),
        other => other.clone(),
    }
}

fn normalize_object(schema: &Value, map: &Map<String, Value>) -> Map<String, Value> {
    let properties = schema.get("properties").and_then(Value::as_object);
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut out = Map::with_capacity(map.len());
    for (key, value) in map {
        let Some(property) = properties.and_then(|p| p.get(key)) else {
            out.insert(key.clone(), value.clone());
            continue;
        };
        if !required.contains(key.as_str()) && is_placeholder(property, value) {
            continue;
        }
        out.insert(key.clone(), normalize_nested(property, value));
    }
    out
}

fn normalize_nested(schema: &Value, value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(normalize_object(schema, map)),
        Value::Array(items) => match schema.get("items") {
            Some(item_schema) => Value::Array(
                items
                    .iter()
                    .map(|item| normalize_nested(item_schema, item))
                    .collect(),
            ),
            None => value.clone(),
        },
        other => other.clone(),
    }
}

/// Whether `value` is how a strict-mode model says "not given" for a property with this schema.
fn is_placeholder(schema: &Value, value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => {
            text.is_empty()
                && declares(schema, "string")
                && schema.get("minLength").and_then(Value::as_u64) != Some(0)
        }
        Value::Array(items) => items.is_empty() && declares(schema, "array"),
        Value::Object(map) => map.is_empty() && declares(schema, "object"),
        Value::Number(number) => match (
            number.as_f64(),
            schema.get("minimum").and_then(Value::as_f64),
        ) {
            (Some(n), Some(minimum)) => n < minimum,
            _ => false,
        },
        Value::Bool(_) => false,
    }
}

/// Whether the schema's `type` is (or includes) `kind`.
fn declares(schema: &Value, kind: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(declared)) => declared == kind,
        Some(Value::Array(declared)) => declared.iter().any(|d| d.as_str() == Some(kind)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shell_like() -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "commands": { "type": "array", "items": { "type": "string" } },
                "cwd": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "background": { "type": "boolean" },
                "headers": { "type": "object" },
                "mode": { "type": "string", "enum": ["a", "b"] }
            }
        })
    }

    #[test]
    fn drops_the_placeholder_for_every_type() {
        let args = json!({
            "command": "ls",
            "commands": [],
            "cwd": "",
            "timeout_secs": 0,
            "headers": {},
            "mode": null
        });
        assert_eq!(
            normalize_args(&shell_like(), &args),
            json!({ "command": "ls" })
        );
    }

    #[test]
    fn keeps_real_values_and_booleans() {
        let args = json!({
            "command": "ls",
            "commands": ["a"],
            "cwd": ".",
            "timeout_secs": 5,
            "background": false,
            "headers": { "x": "y" },
            "mode": "a"
        });
        assert_eq!(normalize_args(&shell_like(), &args), args);
    }

    #[test]
    fn an_empty_command_is_dropped_so_commands_can_stand_alone() {
        let args = json!({ "command": "", "commands": ["ls", "pwd"] });
        assert_eq!(
            normalize_args(&shell_like(), &args),
            json!({ "commands": ["ls", "pwd"] })
        );
    }

    #[test]
    fn required_properties_keep_their_empty_values() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["path", "content", "tags"]
        });
        let args = json!({ "path": "a.txt", "content": "", "tags": [] });
        assert_eq!(normalize_args(&schema, &args), args);
    }

    #[test]
    fn min_length_zero_marks_an_empty_string_as_a_real_value() {
        let schema = json!({
            "type": "object",
            "properties": { "source": { "type": "string", "minLength": 0 } }
        });
        let args = json!({ "source": "" });
        assert_eq!(normalize_args(&schema, &args), args);
    }

    #[test]
    fn zero_survives_unless_the_schema_forbids_it() {
        let schema = json!({
            "type": "object",
            "properties": {
                "context": { "type": "integer" },
                "x": { "type": "integer", "minimum": 0 },
                "limit": { "type": "integer", "minimum": 1 }
            }
        });
        let args = json!({ "context": 0, "x": 0, "limit": 0 });
        assert_eq!(
            normalize_args(&schema, &args),
            json!({ "context": 0, "x": 0 })
        );
    }

    #[test]
    fn unknown_properties_and_untyped_empties_are_left_alone() {
        let schema = json!({
            "type": "object",
            "properties": { "anything": {} }
        });
        let args = json!({ "anything": "", "extra": [], "other": null });
        assert_eq!(normalize_args(&schema, &args), args);
    }

    #[test]
    fn normalises_the_objects_inside_arrays() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "edits": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "old": { "type": "string" },
                            "new": { "type": "string" },
                            "replace_all": { "type": "boolean" }
                        },
                        "required": ["old", "new"]
                    }
                }
            },
            "required": ["edits"]
        });
        let args = json!({
            "path": "a.rs",
            "edits": [{ "path": "", "old": "x", "new": "", "replace_all": false }]
        });
        assert_eq!(
            normalize_args(&schema, &args),
            json!({ "path": "a.rs", "edits": [{ "old": "x", "new": "", "replace_all": false }] })
        );
    }

    #[test]
    fn a_non_object_comes_back_unchanged() {
        assert_eq!(normalize_args(&shell_like(), &json!("ls")), json!("ls"));
        assert_eq!(normalize_args(&shell_like(), &Value::Null), Value::Null);
    }
    /// What a strict-mode model would send for an absent optional property of this schema.
    fn placeholder_for(schema: &Value) -> Option<Value> {
        if declares(schema, "string") {
            return Some(json!(""));
        }
        if declares(schema, "array") {
            return Some(json!([]));
        }
        if declares(schema, "object") {
            return Some(json!({}));
        }
        let strictly_positive = schema.get("minimum").and_then(Value::as_f64) >= Some(1.0);
        (declares(schema, "integer") && strictly_positive).then(|| json!(0))
    }

    #[test]
    fn every_registered_tool_loses_the_placeholders_for_all_its_optional_properties() {
        let registry = crate::ToolRegistry::with_core_tools();
        let mut checked = 0;
        for name in registry.names().map(str::to_string).collect::<Vec<_>>() {
            let schema = registry.get(&name).unwrap().schema();
            let required: HashSet<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut filled = Map::new();
            for (key, property) in schema["properties"].as_object().unwrap() {
                if required.contains(key.as_str()) {
                    continue;
                }
                if let Some(placeholder) = placeholder_for(property) {
                    filled.insert(key.clone(), placeholder);
                    checked += 1;
                } else {
                    // Only a boolean (`false` is an answer) or a count that may be 0 is allowed
                    // to have no placeholder; a property with no declared type would let a
                    // model's empty value through to the tool.
                    assert!(
                        declares(property, "boolean")
                            || declares(property, "integer")
                            || declares(property, "number"),
                        "{name}.{key} declares no type normalisation can read: {property}"
                    );
                }
            }
            assert_eq!(
                normalize_args(&schema, &Value::Object(filled.clone())),
                json!({}),
                "{name}: placeholders left after normalising {filled:?}"
            );
        }
        assert!(checked > 50, "the sweep saw {checked} optional properties");
    }
}
