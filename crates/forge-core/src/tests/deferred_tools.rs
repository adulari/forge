use super::*;

fn spec(name: &str, desc: &str, schema: Value) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: desc.into(),
        schema,
    }
}

fn call(name: &str, args: Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: "c1".into(),
        name: name.into(),
        args,
    }
}

#[test]
fn catalog_glosses_each_deferred_tool_by_its_first_sentence() {
    let deferred = [
        spec(
            "device",
            "Drive an Android emulator or phone: tap, swipe, screenshot, install and much more \
             besides. Second sentence.",
            json!({}),
        ),
        spec("browser", "Headless web browser.", json!({})),
    ];
    let [search, call] = meta_specs(&deferred, false).unwrap();
    assert_eq!(
        (search.name.as_str(), call.name.as_str()),
        (TOOL_SEARCH, TOOL_CALL)
    );
    let desc = search.description;
    assert!(
        desc.contains("browser (Headless web browser); device (Drive an Android"),
        "{desc}"
    );
    assert!(
        desc.contains('…') && !desc.contains("Second sentence"),
        "{desc}"
    );
    assert!(!desc.contains("server__tool"));
    let [with_mcp, _] = meta_specs(&deferred, true).unwrap();
    assert!(with_mcp.description.contains("server__tool"));
    assert!(meta_specs(&[], false).is_none());
}

#[test]
fn search_appends_an_example_built_from_required_fields() {
    let deferred = [spec(
        "web_fetch",
        "Fetch a URL.",
        json!({"type": "object", "properties": {
            "url": {"type": "string"}, "max": {"type": "integer"}, "raw": {"type": "boolean"}
        }, "required": ["url", "max"]}),
    )];
    let out = search(&deferred, "fetch");
    assert!(out.starts_with("## web_fetch\nFetch a URL."), "{out}");
    assert!(
        out.contains(r#"example: tool_call{"arguments":{"max":0,"url":"…"},"name":"web_fetch"}"#),
        "{out}"
    );
    assert!(search(&deferred, "nothing").contains("Available: web_fetch"));
}

#[test]
fn lean_surface_slims_core_and_adds_the_meta_pair() {
    let long = "Run in the background. A long second sentence that must not survive slimming.";
    let core = vec![spec(
        "shell",
        "s",
        json!({"properties": {"background": {"type": "boolean", "description": long}}}),
    )];
    let out = lean_surface(core, &[spec("device", "d", json!({}))], false);
    let names: Vec<&str> = out.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["shell", TOOL_SEARCH, TOOL_CALL]);
    assert_eq!(
        out[0].schema["properties"]["background"]["description"],
        "Run in the background."
    );
}

#[test]
fn unwrap_keeps_the_id_and_accepts_string_or_missing_arguments() {
    assert!(unwrap_tool_call(&call("read_file", json!({}))).is_none());
    let inner = unwrap_tool_call(&call(
        TOOL_CALL,
        json!({"name": "web_fetch", "arguments": {"url": "u"}}),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        (inner.id.as_str(), inner.name.as_str()),
        ("c1", "web_fetch")
    );
    assert_eq!(inner.args, json!({"url": "u"}));
    let stringly = unwrap_tool_call(&call(
        TOOL_CALL,
        json!({"name": "web_fetch", "arguments": "{\"url\":\"u\"}"}),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(stringly.args, json!({"url": "u"}));
    let bare = unwrap_tool_call(&call(TOOL_CALL, json!({"name": "list_sessions"})))
        .unwrap()
        .unwrap();
    assert_eq!(bare.args, json!({}));
    for bad in [
        json!({}),
        json!({"name": "tool_call"}),
        json!({"name": "tool_search"}),
    ] {
        assert!(unwrap_tool_call(&call(TOOL_CALL, bad)).unwrap().is_err());
    }
}

#[test]
fn lean_env_override_wins_both_ways() {
    assert!(lean_enabled_with(true, None));
    assert!(!lean_enabled_with(false, None));
    assert!(lean_enabled_with(false, Some("1")));
    assert!(!lean_enabled_with(true, Some("0")));
    assert!(lean_enabled_with(true, Some("garbage")));
}
