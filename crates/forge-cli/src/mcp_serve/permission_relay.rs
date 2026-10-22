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
    (tool, side_effect, mode): (&str, forge_types::SideEffect, PermissionMode),
    prompt: Prompt<'_>,
) -> Result<(), String> {
    if decision != PermissionDecision::Ask {
        return gate_decision(decision, tool, mode);
    }
    let Ok(sink) = std::env::var(forge_provider::SUBAGENT_SINK_ENV) else {
        return gate_decision(decision, tool, mode);
    };
    let question = Question {
        tool,
        side_effect,
        prompt,
    };
    match relay_permission(&sink, &question, PERMISSION_RELAY_WAIT).await {
        RelayAnswer::Allowed => Ok(()),
        RelayAnswer::Denied(None) => Err(format!(
            "denied by the user: {tool}. Do not retry it; say what you would have done instead."
        )),
        RelayAnswer::Denied(Some(note)) => {
            Err(format!("denied by the user: {tool}. The user said: {note}"))
        }
        RelayAnswer::NoAnswer => Err(format!(
            "denied: {tool} needed the user's confirmation and no answer came within {} minutes.",
            PERMISSION_RELAY_WAIT.as_secs() / 60
        )),
        RelayAnswer::Unavailable => gate_decision(decision, tool, mode),
    }
}

/// Run the `PermissionRequest` hooks for a bridged call that would ask. Returns the settled
/// decision and the (possibly rewritten) arguments, or the refusal text the model reads.
pub(super) async fn permission_hooks(
    hooks: &[forge_config::HookConfig],
    rules: &[PermissionRule],
    (tool, side_effect, mode): (&str, forge_types::SideEffect, PermissionMode),
    decision: PermissionDecision,
    args: Value,
) -> Result<(PermissionDecision, Value), String> {
    if decision != PermissionDecision::Ask
        || !hooks
            .iter()
            .any(|h| h.event == forge_config::HookEvent::PermissionRequest)
    {
        return Ok((decision, args));
    }
    let payload = serde_json::json!({ "tool": tool, "args": args, "permission_mode": mode.key() })
        .to_string();
    let outcome = hooks::run_permission_hooks(hooks, tool, &payload).await;
    match outcome.decision {
        Some(hooks::PermissionHookDecision::Allow { updated_input }) => {
            let args = match updated_input {
                None => args,
                Some(new_args) => {
                    if permission::decide(
                        PermissionMode::Bypass,
                        side_effect,
                        tool,
                        &new_args,
                        rules,
                    ) == PermissionDecision::Deny
                    {
                        return Err(permission::denial_note(tool, &new_args).unwrap_or_else(
                            || "rewritten input is denied by a permission rule".to_string(),
                        ));
                    }
                    new_args
                }
            };
            Ok((PermissionDecision::Allow, args))
        }
        Some(hooks::PermissionHookDecision::Deny { message, .. }) => {
            Err(format!("denied by PermissionRequest hook: {message}"))
        }
        None => Ok((decision, args)),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RelayAnswer {
    Allowed,
    /// With the note the user typed, if any.
    Denied(Option<String>),
    NoAnswer,
    Unavailable,
}

/// What the parent's prompt shows about the call: its arguments, and for a write the change it
/// would make. Without them the parent could only ask "Allow Edit of ?" with no path and no diff,
/// where a native call shows both.
#[derive(Default)]
pub(super) struct Prompt<'a> {
    pub(super) args: Option<&'a Value>,
    pub(super) diff: Option<forge_types::FileDiff>,
}

struct Question<'a> {
    tool: &'a str,
    side_effect: forge_types::SideEffect,
    prompt: Prompt<'a>,
}

/// Longest argument string relayed with a question. The prompt needs the path or the command; a
/// write's full content already travels in the diff.
const RELAYED_ARG_CHARS: usize = 2_000;

fn relayed_args(args: &Value) -> Value {
    match args {
        Value::String(text) if text.chars().count() > RELAYED_ARG_CHARS => {
            Value::String(forge_types::truncate_ellipsis(text, RELAYED_ARG_CHARS))
        }
        Value::Array(items) => Value::Array(items.iter().map(relayed_args).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), relayed_args(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Post one permission question to the parent's sink and wait for its answer file.
async fn relay_permission(
    sink: &str,
    question: &Question<'_>,
    wait: std::time::Duration,
) -> RelayAnswer {
    let answer_path =
        std::path::PathBuf::from(format!("{sink}.permission-{}", forge_types::new_id()));
    let mut record = serde_json::json!({
        "k": "permission",
        "tool": question.tool,
        "side_effect": question.side_effect,
        "answer": answer_path,
    });
    if let Some(args) = question.prompt.args {
        record["args"] = relayed_args(args);
    }
    if let Some(diff) = &question.prompt.diff {
        record["diff"] = serde_json::to_value(diff).unwrap_or(Value::Null);
    }
    if append_sink_record(sink, &record).is_err() {
        return RelayAnswer::Unavailable;
    }
    let deadline = std::time::Instant::now() + wait;
    loop {
        if let Ok(text) = std::fs::read_to_string(&answer_path) {
            if !text.trim().is_empty() {
                let _ = std::fs::remove_file(&answer_path);
                return parse_answer(&text);
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = std::fs::remove_file(&answer_path);
            return RelayAnswer::NoAnswer;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}

/// The parent's answer file: a decision word, then on a deny the user's note.
fn parse_answer(text: &str) -> RelayAnswer {
    let text = text.trim();
    let (word, note) = text.split_once('\n').unwrap_or((text, ""));
    match word.trim() {
        "allow" | "always" | "session" => RelayAnswer::Allowed,
        _ => {
            let note = note.trim();
            RelayAnswer::Denied((!note.is_empty()).then(|| note.to_string()))
        }
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

    fn question() -> Question<'static> {
        Question {
            tool: "write_file",
            side_effect: forge_types::SideEffect::Write,
            prompt: Prompt::default(),
        }
    }

    /// The question names the call and carries the write's diff, so the parent's prompt shows
    /// the path and the change; long argument strings are cut, the diff is not.
    #[tokio::test]
    async fn a_relayed_question_carries_the_arguments_and_the_diff() {
        let dir = std::env::temp_dir().join(format!("forge-relay-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sink = dir.join("sink.jsonl");
        let args = serde_json::json!({"path": "src/a.rs", "old": "x", "new": "y".repeat(5_000)});
        let diff = forge_types::FileDiff {
            path: "src/a.rs".into(),
            kind: forge_types::DiffKind::Modified,
            old: Some("x\n".into()),
            new: Some("y\n".into()),
            lang: Some("rust".into()),
            binary: false,
        };
        let question = Question {
            tool: "edit_file",
            side_effect: forge_types::SideEffect::Write,
            prompt: Prompt {
                args: Some(&args),
                diff: Some(diff.clone()),
            },
        };
        let answer = relay_permission(
            sink.to_str().unwrap(),
            &question,
            std::time::Duration::from_millis(200),
        )
        .await;
        assert_eq!(answer, RelayAnswer::NoAnswer);
        let line = std::fs::read_to_string(&sink).unwrap();
        let record: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(record["args"]["path"], "src/a.rs");
        assert!(record["args"]["new"].as_str().unwrap().chars().count() <= RELAYED_ARG_CHARS);
        let relayed: forge_types::FileDiff =
            serde_json::from_value(record["diff"].clone()).unwrap();
        assert_eq!(relayed, diff);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
            ("session", RelayAnswer::Allowed),
            ("deny", RelayAnswer::Denied(None)),
            (
                "deny\nuse rg instead",
                RelayAnswer::Denied(Some("use rg instead".into())),
            ),
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
            let got =
                relay_permission(&sink_str, &question(), std::time::Duration::from_secs(10)).await;
            parent.await.unwrap();
            assert_eq!(got, expected, "reply {reply}");
        }

        let _ = std::fs::remove_file(&sink);
        let unanswered = relay_permission(
            &sink_str,
            &question(),
            std::time::Duration::from_millis(300),
        )
        .await;
        assert_eq!(unanswered, RelayAnswer::NoAnswer);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_word_or_a_note_after_allow_never_turns_into_an_allow() {
        assert_eq!(parse_answer("allowed"), RelayAnswer::Denied(None));
        assert_eq!(
            parse_answer("maybe\nlater"),
            RelayAnswer::Denied(Some("later".into()))
        );
        assert_eq!(parse_answer("always\n"), RelayAnswer::Allowed);
    }
}
