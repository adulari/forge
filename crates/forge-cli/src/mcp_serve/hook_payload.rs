use serde_json::Value;

/// The stdin payload a hook sees for a bridged call: the same fields the direct path sends
/// (`cwd`, `session_id`) so one hook script works on both.
pub(super) fn bridge_hook_payload(
    name: &str,
    args: &Value,
    result: Option<(&str, bool)>,
) -> String {
    let mut payload = serde_json::json!({
        "tool": name,
        "args": args,
        "cwd": std::env::current_dir().ok().map(|cwd| cwd.display().to_string()),
        "session_id": std::env::var(forge_core::snapshot::ENV_SESSION).unwrap_or_default(),
    });
    if let Some((out, ok)) = result {
        payload["result"] = Value::from(out);
        payload["ok"] = Value::from(ok);
    }
    payload.to_string()
}
