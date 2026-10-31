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
    let normalized = match args {
        Value::Object(map) => Value::Object(normalize_object(schema, map)),
        other => other.clone(),
    };
    if crate::harness::arg_coercion() {
        coerce_args(schema, &normalized)
    } else {
        normalized
    }
}

/// `args` with each property coerced to the type its schema declares, for the mistakes models make
/// most: a JSON array or object sent as a string, a lone value where an array is wanted, `"true"`
/// or `"12"` for a boolean or number, a number where a string is wanted. A value that cannot be
/// coerced is left as sent, so validation reports it.
pub fn coerce_args(schema: &Value, args: &Value) -> Value {
    coerce_value(schema, args)
}

fn coerce_value(schema: &Value, value: &Value) -> Value {
    if declares(schema, "array") {
        let items = schema.get("items").cloned().unwrap_or(Value::Null);
        return match value {
            Value::Array(list) => {
                Value::Array(list.iter().map(|v| coerce_value(&items, v)).collect())
            }
            Value::String(text) => match serde_json::from_str::<Value>(text.trim()) {
                Ok(Value::Array(list)) => {
                    Value::Array(list.iter().map(|v| coerce_value(&items, v)).collect())
                }
                _ if !text.trim().is_empty() => Value::Array(vec![coerce_value(&items, value)]),
                _ => value.clone(),
            },
            Value::Object(_) | Value::Number(_) | Value::Bool(_) => {
                Value::Array(vec![coerce_value(&items, value)])
            }
            Value::Null => value.clone(),
        };
    }
    if declares(schema, "object") || schema.get("properties").is_some() {
        let parsed;
        let value = match value {
            Value::String(text) => match serde_json::from_str::<Value>(text.trim()) {
                Ok(v @ Value::Object(_)) => {
                    parsed = v;
                    &parsed
                }
                _ => value,
            },
            _ => value,
        };
        let (Value::Object(map), Some(props)) =
            (value, schema.get("properties").and_then(Value::as_object))
        else {
            return value.clone();
        };
        return Value::Object(
            map.iter()
                .map(|(k, v)| match props.get(k) {
                    Some(prop) => (k.clone(), coerce_value(prop, v)),
                    None => (k.clone(), v.clone()),
                })
                .collect(),
        );
    }
    if declares(schema, "boolean") {
        if let Value::String(text) = value {
            match text.trim().to_ascii_lowercase().as_str() {
                "true" => return Value::Bool(true),
                "false" => return Value::Bool(false),
                _ => {}
            }
        }
    } else if declares(schema, "integer") {
        if let Value::String(text) = value {
            if let Ok(n) = text.trim().parse::<i64>() {
                return Value::from(n);
            }
        }
        if let Some(f) = value.as_f64().filter(|_| value.is_f64()) {
            if f.fract() == 0.0 && f.abs() < 9e15 {
                return Value::from(f as i64);
            }
        }
    } else if declares(schema, "number") {
        if let Value::String(text) = value {
            if let Ok(n) = text.trim().parse::<f64>() {
                if let Some(n) = serde_json::Number::from_f64(n) {
                    return Value::Number(n);
                }
            }
        }
    } else if declares(schema, "string") && !declares(schema, "number") {
        match value {
            Value::Number(n) => return Value::String(n.to_string()),
            Value::Bool(b) => return Value::String(b.to_string()),
            _ => {}
        }
    }
    value.clone()
}

/// Type problems that coercion could not fix, one line each (`start_line: expected integer, got
/// string`). Only top-level properties; empty when the arguments fit the schema.
pub fn type_mismatches(schema: &Value, args: &Value) -> Vec<String> {
    let (Some(map), Some(props)) = (
        args.as_object(),
        schema.get("properties").and_then(Value::as_object),
    ) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, value) in map {
        let Some(prop) = props.get(key) else { continue };
        let Some(Value::String(kind)) = prop.get("type") else {
            continue;
        };
        let actual = match value {
            Value::Null => continue,
            Value::Bool(_) => "boolean",
            Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        let fits = actual == kind || (kind == "number" && actual == "integer");
        if !fits {
            out.push(format!("{key}: expected {kind}, got {actual}"));
        }
    }
    out
}

/// The tool's parameters on one short line each (`path: string (required) - description`), for
/// putting in an error so the model can repair the call without another schema lookup.
pub fn compact_schema(schema: &Value) -> String {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return String::new();
    };
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut lines = Vec::new();
    for (key, prop) in props {
        let kind = match prop.get("type") {
            Some(Value::String(t)) if t == "array" => {
                let item = prop
                    .get("items")
                    .and_then(|i| i.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("any");
                format!("array of {item}")
            }
            Some(Value::String(t)) => t.clone(),
            _ => "any".to_string(),
        };
        let mark = if required.contains(key.as_str()) {
            " (required)"
        } else {
            ""
        };
        let desc: String = prop
            .get("description")
            .and_then(Value::as_str)
            .map(|d| d.chars().take(90).collect())
            .unwrap_or_default();
        let sep = if desc.is_empty() { "" } else { " - " };
        lines.push(format!("  {key}: {kind}{mark}{sep}{desc}"));
    }
    lines.join("\n")
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

#[cfg(test)]
mod coercion_tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": { "type": "array", "items": { "type": "string" } },
                "edits": { "type": "array", "items": { "type": "object", "properties": {
                    "old": { "type": "string" }, "line": { "type": "integer" } } } },
                "background": { "type": "boolean" },
                "timeout_secs": { "type": "integer" },
                "ratio": { "type": "number" },
                "name": { "type": "string" },
                "opts": { "type": "object", "properties": { "n": { "type": "integer" } } }
            },
            "required": ["paths"]
        })
    }

    #[test]
    fn strings_become_arrays_booleans_and_numbers() {
        let args = json!({
            "paths": "[\"a.rs\", \"b.rs\"]", "background": "true",
            "timeout_secs": "30", "ratio": "0.5", "name": 7
        });
        assert_eq!(
            coerce_args(&schema(), &args),
            json!({ "paths": ["a.rs", "b.rs"], "background": true,
                    "timeout_secs": 30, "ratio": 0.5, "name": "7" })
        );
    }

    #[test]
    fn a_lone_value_becomes_a_one_element_array() {
        let args = json!({ "paths": "a.rs", "edits": { "old": "x", "line": "3" } });
        assert_eq!(
            coerce_args(&schema(), &args),
            json!({ "paths": ["a.rs"], "edits": [{ "old": "x", "line": 3 }] })
        );
    }

    #[test]
    fn json_in_a_string_becomes_an_object_and_whole_floats_become_integers() {
        let args = json!({ "paths": [], "opts": "{\"n\": \"4\"}", "timeout_secs": 5.0 });
        assert_eq!(
            coerce_args(&schema(), &args),
            json!({ "paths": [], "opts": { "n": 4 }, "timeout_secs": 5 })
        );
    }

    #[test]
    fn unfixable_values_are_left_for_validation_to_report() {
        let args = json!({ "paths": ["a"], "timeout_secs": "soon", "background": "maybe" });
        let out = coerce_args(&schema(), &args);
        assert_eq!(out, args);
        let problems = type_mismatches(&schema(), &out);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems
            .iter()
            .any(|p| p.contains("timeout_secs: expected integer, got string")));
    }

    #[test]
    fn compact_schema_lists_required_and_types() {
        let text = compact_schema(&schema());
        assert!(text.contains("paths: array of string (required)"), "{text}");
        assert!(text.contains("background: boolean"), "{text}");
    }
}
