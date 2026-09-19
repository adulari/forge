//! Relaying a bridged tool's `Ask` permission decision to the parent session's prompt.

use super::*;

/// How long a relayed permission question waits for the user. Below the bridges' own tool
/// timeout (`BRIDGE_TOOL_TIMEOUT_SECS`) so the refusal comes from here, with a reason.
const PERMISSION_RELAY_WAIT: std::time::Duration = std::time::Duration::from_secs(600);

/// The permission gate for a bridged call. An `Ask` used to be an automatic refusal — a bridge
/// has no terminal to prompt on — so in the default temper every edit a claude/codex turn tried
/// failed, on the desktop and the phone alike. When the parent session handed us its event sink,
/// the question is relayed there and the user answers it in the parent's own prompt.
pub(super) async fn gate(
    decision: PermissionDecision,
    tool: &str,
    side_effect: forge_types::SideEffect,
    mode: PermissionMode,
) -> Result<(), String> {
    if decision != PermissionDecision::Ask {
        return gate_decision(decision, tool, mode);
    }
    let Ok(sink) = std::env::var(forge_provider::SUBAGENT_SINK_ENV) else {
        return gate_decision(decision, tool, mode);
    };
    match relay_permission(&sink, tool, side_effect, PERMISSION_RELAY_WAIT).await {
        RelayAnswer::Allowed => Ok(()),
        RelayAnswer::Denied => Err(format!(
            "denied by the user: {tool}. Do not retry it; say what you would have done instead."
        )),
        RelayAnswer::NoAnswer => Err(format!(
            "denied: {tool} needed the user's confirmation and no answer came within {} minutes.",
            PERMISSION_RELAY_WAIT.as_secs() / 60
        )),
        RelayAnswer::Unavailable => gate_decision(decision, tool, mode),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RelayAnswer {
    Allowed,
    Denied,
    NoAnswer,
    Unavailable,
}

/// Post one permission question to the parent's sink and wait for its answer file.
async fn relay_permission(
    sink: &str,
    tool: &str,
    side_effect: forge_types::SideEffect,
    wait: std::time::Duration,
) -> RelayAnswer {
    let answer_path =
        std::path::PathBuf::from(format!("{sink}.permission-{}", forge_types::new_id()));
    let record = serde_json::json!({
        "k": "permission",
        "tool": tool,
        "side_effect": side_effect,
        "answer": answer_path,
    });
    if append_sink_record(sink, &record).is_err() {
        return RelayAnswer::Unavailable;
    }
    let deadline = std::time::Instant::now() + wait;
    loop {
        if let Ok(text) = std::fs::read_to_string(&answer_path) {
            let answer = text.trim();
            if !answer.is_empty() {
                let _ = std::fs::remove_file(&answer_path);
                return if matches!(answer, "allow" | "always") {
                    RelayAnswer::Allowed
                } else {
                    RelayAnswer::Denied
                };
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = std::fs::remove_file(&answer_path);
            return RelayAnswer::NoAnswer;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

/// Resolve a broker decision on the bridge path, which has no TTY and no presenter.
///
/// `Deny` refuses, and so does an `Ask` that could not be relayed (no parent sink): this process
/// cannot prompt, and running the call anyway
/// would make the Ask tempers (`default`, and `plan`'s residual asks) behave exactly like
/// `bypass` — a silent privilege escalation for anyone who deliberately picked a stricter
/// posture. The refusal text names the temper so the user can escalate on purpose.
pub(super) fn gate_decision(
    decision: PermissionDecision,
    tool: &str,
    mode: PermissionMode,
) -> Result<(), String> {
    match decision {
        PermissionDecision::Allow => Ok(()),
        PermissionDecision::Deny => Err(format!("denied by Forge permission policy: {tool}")),
        PermissionDecision::Ask => Err(format!(
            "denied by Forge permission policy: {tool} needs confirmation in the `{}` temper and \
             this bridged session has no way to prompt. Ask the user to switch to auto-edit or \
             full, or add an allow rule for this tool.",
            mode.key()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay's wire contract with the parent: one `permission` line on the sink, answered by
    /// writing the decision to the file it names.
    #[tokio::test]
    async fn a_relayed_permission_waits_for_the_parents_answer() {
        let dir = std::env::temp_dir().join(format!("forge-relay-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sink = dir.join("sink.jsonl");
        let sink_str = sink.to_str().unwrap().to_owned();

        for (reply, expected) in [
            ("allow", RelayAnswer::Allowed),
            ("always", RelayAnswer::Allowed),
            ("deny", RelayAnswer::Denied),
        ] {
            let _ = std::fs::remove_file(&sink);
            let parent_sink = sink.clone();
            let parent = tokio::spawn(async move {
                loop {
                    if let Ok(text) = std::fs::read_to_string(&parent_sink) {
                        if let Some(line) = text.lines().last() {
                            let record: serde_json::Value = serde_json::from_str(line).unwrap();
                            assert_eq!(record["k"], "permission");
                            assert_eq!(record["tool"], "write_file");
                            let answer = record["answer"].as_str().unwrap().to_owned();
                            std::fs::write(answer, reply).unwrap();
                            return;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            });
            let got = relay_permission(
                &sink_str,
                "write_file",
                forge_types::SideEffect::Write,
                std::time::Duration::from_secs(10),
            )
            .await;
            parent.await.unwrap();
            assert_eq!(got, expected, "reply {reply}");
        }

        let _ = std::fs::remove_file(&sink);
        let unanswered = relay_permission(
            &sink_str,
            "write_file",
            forge_types::SideEffect::Write,
            std::time::Duration::from_millis(300),
        )
        .await;
        assert_eq!(unanswered, RelayAnswer::NoAnswer);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
