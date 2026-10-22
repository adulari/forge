//! `async: true` command hooks (Claude-Code parity): the event never waits for the hook, and when
//! the background process exits, the `additionalContext` and `systemMessage` of its JSON answer
//! reach the model at the start of the session's next turn. Nothing else from an async hook has an
//! effect: the action it could have blocked has already happened.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use forge_config::HookConfig;
use serde_json::Value;

/// Claude Code does not time out an async hook; this only stops a wedged one living forever.
const ASYNC_HOOK_CAP_SECS: u64 = 6 * 60 * 60;

fn pending() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static PENDING: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// Start `hook` in the background with `payload` on stdin. Outside a tokio runtime it is skipped.
pub(crate) fn spawn(hook: HookConfig, payload: String) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let session = serde_json::from_str::<Value>(&payload)
        .ok()
        .and_then(|v| {
            v.get("session_id")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .unwrap_or_default();
    runtime.spawn(async move {
        let hook = HookConfig {
            run_async: false,
            timeout_secs: ASYNC_HOOK_CAP_SECS,
            ..hook
        };
        match crate::hooks_exec::run_command(&hook, &payload).await {
            Ok((_, stdout, _)) => {
                let messages = messages_from(&stdout);
                if !messages.is_empty() {
                    let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
                    map.entry(session).or_default().extend(messages);
                }
            }
            Err(error) => tracing::debug!(%error, command = %hook.command, "async hook failed"),
        }
    });
}

/// What finished async hooks left for `session_id`, oldest first; each is delivered once.
pub(crate) fn take(session_id: &str) -> Vec<String> {
    pending()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(session_id)
        .unwrap_or_default()
}

/// The deliverable text of an async hook's stdout: its JSON answer printed alone (possibly over
/// several lines) or on one line of its own.
fn messages_from(stdout: &str) -> Vec<String> {
    let answer = serde_json::from_str::<Value>(stdout.trim())
        .ok()
        .filter(Value::is_object)
        .or_else(|| {
            stdout
                .lines()
                .rev()
                .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
                .find(Value::is_object)
        });
    let Some(answer) = answer else {
        return Vec::new();
    };
    let text = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let context = text(
        answer
            .get("hookSpecificOutput")
            .and_then(|h| h.get("additionalContext")),
    )
    .or_else(|| text(answer.get("additionalContext")));
    context
        .into_iter()
        .chain(text(answer.get("systemMessage")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_json_answer_may_be_alone_or_on_its_own_line() {
        assert_eq!(
            messages_from("{\n  \"systemMessage\": \"tests pass\"\n}\n"),
            vec!["tests pass"]
        );
        assert_eq!(
            messages_from(
                "running...\n{\"hookSpecificOutput\":{\"additionalContext\":\"2 failures\"}}\n"
            ),
            vec!["2 failures"]
        );
        assert!(messages_from("plain text only").is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_async_hook_does_not_hold_the_event_and_reports_later() {
        let hook = HookConfig {
            run_async: true,
            ..HookConfig::new(
                forge_config::HookEvent::PostToolUse,
                r#"sleep 1; echo '{"systemMessage":"background check done"}'"#,
            )
        };
        let started = std::time::Instant::now();
        let out = crate::hooks::run_hooks(
            &[hook],
            forge_config::HookEvent::PostToolUse,
            "shell",
            r#"{"tool":"shell","args":{},"session_id":"async-sess-1"}"#,
        )
        .await;
        assert!(started.elapsed() < std::time::Duration::from_millis(900));
        assert!(out.notes.is_empty() && out.injected_context.is_empty());
        let mut delivered = Vec::new();
        for _ in 0..60 {
            delivered = take("async-sess-1");
            if !delivered.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(delivered, vec!["background check done"]);
        assert!(take("async-sess-1").is_empty(), "delivered once");
    }
}
