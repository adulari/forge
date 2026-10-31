//! A small, dependency-free JSON Schema subset for model-authored result schemas.
//!
//! The parent agent writes a schema, a child returns data, and both sides are untrusted models, so
//! the subset is deliberately narrow: `type`, `properties`, `required`, `additionalProperties`
//! (bool), `items`, `enum`, plus the `description`/`title` annotations. Any other keyword is
//! rejected by name by [`check_supported`] rather than silently ignored.

use serde_json::{Map, Value};

pub const MAX_SCHEMA_BYTES: usize = 4096;
pub const MAX_SCHEMA_DEPTH: usize = 8;

const ALLOWED_KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "description",
    "title",
];

const TYPE_NAMES: &[&str] = &[
    "object", "array", "string", "number", "integer", "boolean", "null",
];

/// One violation, with a `$.files[2].line` style path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub path: String,
    pub message: String,
}

/// Whether `schema` uses only the supported subset and has a root of `type: "object"`.
pub fn check_supported(schema: &Value) -> Result<(), String> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(
            "the root of an output schema must be {\"type\":\"object\"}; to return a list, wrap it: \
             {\"type\":\"object\",\"properties\":{\"items\":{\"type\":\"array\",\"items\":{...}}}}"
                .to_string(),
        );
    }
    let size = schema.to_string().len();
    if size > MAX_SCHEMA_BYTES {
        return Err(format!(
            "the output schema is {size} bytes; the limit is {MAX_SCHEMA_BYTES}. Use a flatter, smaller schema"
        ));
    }
    check_node(schema, "$", 1)
}

fn check_node(node: &Value, path: &str, depth: usize) -> Result<(), String> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(format!(
            "the output schema nests deeper than {MAX_SCHEMA_DEPTH} levels at {path}"
        ));
    }
    let Some(map) = node.as_object() else {
        return Err(format!("{path}: a schema node must be an object"));
    };
    for key in map.keys() {
        if !ALLOWED_KEYWORDS.contains(&key.as_str()) {
            return Err(format!(
                "{path}: unsupported schema keyword `{key}`; supported: {}",
                ALLOWED_KEYWORDS.join(", ")
            ));
        }
    }
    if let Some(t) = map.get("type") {
        let names: Vec<&Value> = match t {
            Value::Array(a) => a.iter().collect(),
            other => vec![other],
        };
        if names.is_empty() {
            return Err(format!("{path}: `type` must not be an empty array"));
        }
        for name in names {
            if !name.as_str().is_some_and(|n| TYPE_NAMES.contains(&n)) {
                return Err(format!(
                    "{path}: unsupported `type` {name}; use one of {}",
                    TYPE_NAMES.join(", ")
                ));
            }
        }
    }
    if let Some(props) = map.get("properties") {
        let Some(props) = props.as_object() else {
            return Err(format!("{path}.properties must be an object"));
        };
        for (key, child) in props {
            check_node(child, &format!("{path}.{key}"), depth + 1)?;
        }
    }
    if let Some(req) = map.get("required") {
        let ok = req
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string));
        if !ok {
            return Err(format!("{path}.required must be an array of strings"));
        }
    }
    if let Some(ap) = map.get("additionalProperties") {
        if !ap.is_boolean() {
            return Err(format!("{path}.additionalProperties must be true or false"));
        }
    }
    if let Some(items) = map.get("items") {
        check_node(items, &format!("{path}[]"), depth + 1)?;
    }
    if let Some(e) = map.get("enum") {
        if !e.as_array().is_some_and(|a| !a.is_empty()) {
            return Err(format!("{path}.enum must be a non-empty array"));
        }
    }
    Ok(())
}

fn type_names(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn is_type(name: &str, value: &Value) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value
                    .as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        _ => true,
    }
}

fn describe(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(_) => "boolean".into(),
        Value::Number(_) => "number".into(),
        Value::String(s) => {
            let shown: String = s.chars().take(40).collect();
            format!("string {shown:?}")
        }
        Value::Array(_) => "array".into(),
        Value::Object(_) => "object".into(),
    }
}

/// Lossless conversions of the shape mistakes models make: `"12"` to 12, `"true"` to true,
/// JSON-in-a-string to the object or array it holds, and a lone scalar to a one-item array.
/// Recursive; a value that does not convert exactly is returned unchanged for `validate` to flag.
pub fn coerce(schema: &Value, value: &Value) -> Value {
    let types = type_names(schema);
    if types.is_empty() {
        return value.clone();
    }
    if types.iter().any(|t| is_type(t, value)) {
        return match value {
            Value::Object(map) => Value::Object(coerce_object(schema, map)),
            Value::Array(items) => Value::Array(coerce_items(schema, items)),
            other => other.clone(),
        };
    }
    if let Value::String(s) = value {
        let trimmed = s.trim();
        for t in &types {
            let converted = match *t {
                "integer" => trimmed.parse::<i64>().ok().map(Value::from),
                "number" => trimmed
                    .parse::<f64>()
                    .ok()
                    .filter(|f| f.is_finite())
                    .and_then(|f| {
                        trimmed
                            .parse::<i64>()
                            .ok()
                            .map(Value::from)
                            .or_else(|| serde_json::Number::from_f64(f).map(Value::Number))
                    }),
                "boolean" => match trimmed {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    _ => None,
                },
                "object" | "array" => serde_json::from_str::<Value>(trimmed)
                    .ok()
                    .filter(|v| is_type(t, v)),
                _ => None,
            };
            if let Some(v) = converted {
                return coerce(schema, &v);
            }
        }
    }
    if types.contains(&"array") && !value.is_null() {
        return Value::Array(coerce_items(schema, std::slice::from_ref(value)));
    }
    value.clone()
}

fn coerce_object(schema: &Value, map: &Map<String, Value>) -> Map<String, Value> {
    let props = schema.get("properties").and_then(Value::as_object);
    map.iter()
        .map(|(k, v)| {
            let v = match props.and_then(|p| p.get(k)) {
                Some(child) => coerce(child, v),
                None => v.clone(),
            };
            (k.clone(), v)
        })
        .collect()
}

fn coerce_items(schema: &Value, items: &[Value]) -> Vec<Value> {
    match schema.get("items") {
        Some(item_schema) => items.iter().map(|i| coerce(item_schema, i)).collect(),
        None => items.to_vec(),
    }
}

/// Every violation of `schema` by `value`, at most `max`.
pub fn validate(schema: &Value, value: &Value, max: usize) -> Vec<SchemaError> {
    let mut errors = Vec::new();
    walk(schema, value, "$", max, &mut errors);
    errors
}

fn push(errors: &mut Vec<SchemaError>, max: usize, path: &str, message: String) {
    if errors.len() < max {
        errors.push(SchemaError {
            path: path.to_string(),
            message,
        });
    }
}

fn walk(schema: &Value, value: &Value, path: &str, max: usize, errors: &mut Vec<SchemaError>) {
    if errors.len() >= max {
        return;
    }
    let types = type_names(schema);
    if !types.is_empty() && !types.iter().any(|t| is_type(t, value)) {
        push(
            errors,
            max,
            path,
            format!("expected {}, got {}", types.join(" or "), describe(value)),
        );
        return;
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            let list: Vec<String> = allowed.iter().map(Value::to_string).collect();
            push(
                errors,
                max,
                path,
                format!("expected one of {}, got {value}", list.join(", ")),
            );
            return;
        }
    }
    match value {
        Value::Object(map) => {
            if let Some(required) = schema.get("required").and_then(Value::as_array) {
                for key in required.iter().filter_map(Value::as_str) {
                    if !map.contains_key(key) {
                        push(
                            errors,
                            max,
                            &format!("{path}.{key}"),
                            "required property is missing".to_string(),
                        );
                    }
                }
            }
            let props = schema.get("properties").and_then(Value::as_object);
            let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
            for (key, v) in map {
                match props.and_then(|p| p.get(key)) {
                    Some(child) => walk(child, v, &format!("{path}.{key}"), max, errors),
                    None if closed => push(
                        errors,
                        max,
                        &format!("{path}.{key}"),
                        "unexpected property (additionalProperties is false)".to_string(),
                    ),
                    None => {}
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for (i, v) in items.iter().enumerate() {
                    walk(item_schema, v, &format!("{path}[{i}]"), max, errors);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn root_must_be_object_and_says_how_to_wrap() {
        let err = check_supported(&json!({"type":"array","items":{"type":"string"}})).unwrap_err();
        assert!(err.contains("wrap"), "{err}");
    }

    #[test]
    fn unsupported_keywords_are_named() {
        for kw in [
            "$ref", "anyOf", "oneOf", "allOf", "pattern", "minimum", "nullable",
        ] {
            let schema = json!({"type":"object","properties":{"a":{"type":"string", kw: 1}}});
            let err = check_supported(&schema).unwrap_err();
            assert!(err.contains(&format!("`{kw}`")), "{kw}: {err}");
        }
    }

    #[test]
    fn oversize_and_deep_schemas_are_rejected() {
        let big = json!({"type":"object","properties":{"a":{"type":"string","description":"x".repeat(5000)}}});
        assert!(check_supported(&big).unwrap_err().contains("bytes"));
        let mut deep = json!({"type":"string"});
        for _ in 0..9 {
            deep = json!({"type":"object","properties":{"a":deep}});
        }
        assert!(check_supported(&deep).unwrap_err().contains("deeper"));
        let mut ok = json!({"type":"string"});
        for _ in 0..7 {
            ok = json!({"type":"object","properties":{"a":ok}});
        }
        assert!(check_supported(&ok).is_ok());
    }

    #[test]
    fn supported_schema_passes() {
        let s = json!({"type":"object","title":"t","properties":{
            "v":{"type":["string","null"],"enum":["a","b",null]},
            "files":{"type":"array","items":{"type":"object","properties":{"line":{"type":"integer"}},"required":["line"]}}
        },"required":["v"],"additionalProperties":false});
        assert!(check_supported(&s).is_ok());
    }

    #[test]
    fn validate_reports_precise_paths() {
        let s = json!({"type":"object","properties":{
            "files":{"type":"array","items":{"type":"object","properties":{"line":{"type":"integer"}}}}
        },"required":["files","verdict"]});
        let v = json!({"files":[{"line":1},{"line":2},{"line":"x"}]});
        let errs = validate(&s, &v, 8);
        let paths: Vec<&str> = errs.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"$.files[2].line"), "{paths:?}");
        assert!(paths.contains(&"$.verdict"), "{paths:?}");
    }

    #[test]
    fn validate_additional_enum_integer_and_type_arrays() {
        let s = json!({"type":"object","properties":{
            "n":{"type":"integer"},"e":{"enum":["x","y"]},"s":{"type":["string","null"]}
        },"additionalProperties":false});
        assert!(validate(&s, &json!({"n":3.0,"e":"x","s":null}), 8).is_empty());
        assert_eq!(validate(&s, &json!({"n":3.5}), 8).len(), 1);
        assert_eq!(validate(&s, &json!({"e":"z"}), 8).len(), 1);
        assert_eq!(validate(&s, &json!({"s":4}), 8).len(), 1);
        assert_eq!(validate(&s, &json!({"extra":1}), 8).len(), 1);
    }

    #[test]
    fn validate_caps_errors() {
        let s =
            json!({"type":"object","properties":{"a":{"type":"array","items":{"type":"integer"}}}});
        let v = json!({"a":["x","x","x","x","x"]});
        assert_eq!(validate(&s, &v, 3).len(), 3);
    }

    #[test]
    fn coerce_lossless_cases() {
        let s = json!({"type":"object","properties":{
            "n":{"type":"integer"},"b":{"type":"boolean"},
            "o":{"type":"object","properties":{"k":{"type":"integer"}}},
            "l":{"type":"array","items":{"type":"string"}},
            "arr":{"type":"array","items":{"type":"integer"}}
        }});
        let v = json!({"n":"12","b":"true","o":"{\"k\":\"4\"}","l":"one","arr":"[1,\"2\"]"});
        let out = coerce(&s, &v);
        assert_eq!(
            out,
            json!({"n":12,"b":true,"o":{"k":4},"l":["one"],"arr":[1,2]})
        );
    }

    #[test]
    fn coerce_leaves_lossy_values_alone() {
        let s =
            json!({"type":"object","properties":{"n":{"type":"integer"},"m":{"type":"number"}}});
        let out = coerce(&s, &json!({"n":"1.5","m":"1.5"}));
        assert_eq!(out["n"], json!("1.5"));
        assert_eq!(out["m"], json!(1.5));
    }
}
