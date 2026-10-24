//! The row a `run_code` call gets in the transcript.

use serde_json::Value;

/// What a `run_code` row says in place of the script's source: the tools the script calls, by
/// their verbs (`edit, read`). The calls themselves get rows of their own as they run. `args`
/// is the script, or the call's JSON when the transcript replays one.
pub(super) fn headline(args: &str) -> String {
    let code = serde_json::from_str::<Value>(args)
        .ok()
        .and_then(|v| {
            ["code", "script"]
                .iter()
                .find_map(|k| v.get(*k).and_then(Value::as_str).map(str::to_string))
        })
        .unwrap_or_else(|| args.to_string());
    let mut verbs: Vec<String> = Vec::new();
    for part in code.split("tools.").skip(1) {
        let name: String = part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || !part[name.len()..].trim_start().starts_with('(') {
            continue;
        }
        let verb = super::tool_cards::verb(&name).to_lowercase();
        if !verbs.contains(&verb) {
            verbs.push(verb);
        }
    }
    verbs.join(", ")
}
