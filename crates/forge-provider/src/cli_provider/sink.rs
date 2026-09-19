//! The out-of-band event sink a bridge turn's `forge mcp-serve` child appends to, and the tail
//! that turns each line into a [`StreamEvent`] for the parent session.

use super::*;

/// Parse one line of the subagent sink into a [`StreamEvent`]. Field-tolerant.
pub(super) fn parse_sink_line(line: &str) -> Option<StreamEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match v.get("k").and_then(Value::as_str)? {
        "start" => Some(StreamEvent::SubagentStarted {
            id: s("id"),
            agent: s("agent"),
            task: s("task"),
        }),
        "progress" => Some(StreamEvent::SubagentProgress {
            id: s("id"),
            snippet: s("snippet"),
        }),
        "done" => Some(StreamEvent::SubagentFinished {
            id: s("id"),
            agent: s("agent"),
            ok: v.get("ok").and_then(Value::as_bool).unwrap_or(true),
            summary: s("summary"),
            cost_usd: v.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
        }),
        "tasks" => {
            let tasks = serde_json::from_value(v.get("tasks")?.clone()).ok()?;
            Some(StreamEvent::Tasks(tasks))
        }
        "plan" => {
            let plan = serde_json::from_value(v.get("plan")?.clone()).ok()?;
            Some(StreamEvent::Plan(plan))
        }
        "permission" => {
            let answer = s("answer");
            if answer.is_empty() {
                return None;
            }
            Some(StreamEvent::PermissionRequest {
                tool: s("tool"),
                side_effect: serde_json::from_value(v.get("side_effect")?.clone()).ok()?,
                answer_path: std::path::PathBuf::from(answer),
            })
        }
        _ => None,
    }
}

/// Tail the subagent sink file, forwarding each event over `tx` as it is appended. Runs until
/// aborted by the caller (after the CLI process exits). Tolerant of the file not existing yet.
pub(super) async fn tail_subagent_sink(
    path: std::path::PathBuf,
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    use tokio::io::AsyncBufReadExt;
    // Wait for the file to appear (mcp-serve creates/opens it on first write).
    let file = loop {
        match tokio::fs::File::open(&path).await {
            Ok(f) => break f,
            Err(_) => tokio::time::sleep(Duration::from_millis(40)).await,
        }
    };
    let mut reader = tokio::io::BufReader::new(file);
    let mut buf = String::new();
    loop {
        match reader.read_line(&mut buf).await {
            Ok(0) => tokio::time::sleep(Duration::from_millis(40)).await, // EOF: await more
            Ok(_) if !buf.ends_with('\n') => {
                // Torn read: the reader caught up to the file's current EOF mid-line (no trailing
                // newline yet). Keep the partial bytes buffered — do NOT parse or clear — so the
                // next read_line appends the rest of this same line instead of losing/mis-parsing it.
                continue;
            }
            Ok(_) => {
                if let Some(ev) = parse_sink_line(buf.trim()) {
                    if tx.send(ev).is_err() {
                        break;
                    }
                }
                buf.clear();
            }
            Err(_) => break,
        }
    }
}
