//! Bridge-side records: the subagent activity sink the CLI bridge tails, and the memory and
//! task writes a bridged model makes against its parent session.

use forge_store::Store;
use serde_json::Value;

/// Append one JSON record to the out-of-band subagent sink the CLI bridge tails (if it gave us
/// one via `FORGE_SUBAGENT_SINK`). Used to surface bridge-turn activity (subagents, task-list
/// updates) in the parent Forge TUI live. Best-effort for the CALL (a sink failure must not fail
/// the tool call it decorates), but never silent: an open/write/flush failure is reported as a
/// diagnostic so a full or read-only disk doesn't quietly blind the parent TUI.
pub(super) fn report_to_sink(record: serde_json::Value) {
    let Ok(path) = std::env::var(forge_provider::SUBAGENT_SINK_ENV) else {
        return;
    };
    if let Err(error) = append_sink_record(&path, &record) {
        tracing::warn!(
            path,
            %error,
            "mcp-serve: failed to append bridge activity to the subagent sink; \
             the parent session will not see this event"
        );
    }
}

/// One JSONL append to the sink at `path`, every I/O failure propagated.
pub(super) fn append_sink_record(path: &str, record: &serde_json::Value) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{record}")?;
    f.flush()
}

pub(super) fn parse_bridge_tasks(args: &Value) -> Result<Vec<forge_types::TodoItem>, &'static str> {
    let tasks = forge_core::parse_tasks(args);
    let input_count = args
        .get("tasks")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if input_count > 0 && tasks.is_empty() {
        Err("error: no valid tasks found; each task needs a non-empty title, content, or description")
    } else {
        Ok(tasks)
    }
}

pub(super) fn persist_bridge_memory(
    store: &Store,
    scope: &str,
    kind: &str,
    text: &str,
    session_id: &str,
) -> std::result::Result<(), forge_store::StoreError> {
    // A bridged model's memory is injected into later prompts: mask credentials before storing.
    let masked = forge_core::redact::Redactor::from_environment().redact(text);
    store
        .add_memory(scope, kind, &masked, session_id)
        .map(|_| ())
}

/// Merge and persist a bridge task update for its parent session. Keep the read and write in one
/// fallible seam so an unavailable or invalid Store cannot be reported as a successful update.
pub(super) fn persist_bridge_tasks(
    store: &Store,
    session_id: &str,
    tasks: Vec<forge_types::TodoItem>,
) -> std::result::Result<Vec<forge_types::TodoItem>, forge_store::StoreError> {
    let existing = store.tasks(session_id)?;
    let tasks = forge_core::merge_task_update(&existing, tasks);
    store.set_tasks(session_id, &tasks)?;
    Ok(tasks)
}
