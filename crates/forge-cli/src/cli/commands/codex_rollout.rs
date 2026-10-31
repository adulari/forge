//! Reader for Codex CLI session rollouts (`~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`).
//!
//! A rollout is an append-only log: one `session_meta` line, then `response_item` lines (messages,
//! tool calls and their outputs, reasoning), `turn_context` lines carrying the model, and
//! `event_msg` lines that only mirror what the response items already say. Older rollouts have no
//! `payload` wrapper; the line itself is the item. The file is read, never written.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use forge_types::{Role, ToolCall};
use serde_json::Value;

use super::cc_transcript::{Imported, Normalizer};

/// How much of a rollout's start is read to list it without parsing the whole file.
const HEAD_BYTES: u64 = 256 * 1024;

/// What a listing shows about a rollout.
#[derive(Debug, Clone, Default)]
pub(crate) struct RolloutMeta {
    pub(crate) id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) first_prompt: Option<String>,
}

/// A whole rollout as Forge messages.
pub(crate) struct Rollout {
    pub(crate) meta: RolloutMeta,
    pub(crate) messages: Vec<Imported>,
}

/// `$CODEX_HOME/sessions`, else `~/.codex/sessions`.
pub(crate) fn sessions_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join("sessions"));
    }
    Some(forge_config::codex_dir()?.join("sessions"))
}

/// Every `rollout-*.jsonl` under `dir`, in no particular order.
pub(crate) fn rollout_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && path.extension().is_some_and(|e| e == "jsonl")
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("rollout-"))
            {
                out.push(path);
            }
        }
    }
    out
}

/// The session id in `rollout-2026-10-01T10-00-00-<uuid>.jsonl`: the last five dash-separated
/// groups of the stem.
pub(crate) fn id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.split('-').collect();
    (parts.len() >= 5).then(|| parts[parts.len() - 5..].join("-"))
}

/// A rollout path, or a session id looked up under the sessions directory.
pub(crate) fn locate(session: &str) -> Result<PathBuf> {
    let direct = Path::new(session);
    if direct.is_file() {
        return Ok(direct.to_path_buf());
    }
    if session.contains(['/', '\\']) || session.ends_with(".jsonl") {
        bail!("no such rollout file: {session}");
    }
    let dir = sessions_dir().context("no home directory: cannot locate ~/.codex")?;
    rollout_files(&dir)
        .into_iter()
        .find(|p| {
            id_from_filename(p).is_some_and(|id| id == session)
                || head(p).id.is_some_and(|id| id == session)
        })
        .with_context(|| format!("no Codex session {session} under {}", dir.display()))
}

fn payload(v: &Value) -> &Value {
    match v.get("payload") {
        Some(p) if p.is_object() => p,
        _ => v,
    }
}

fn kind(v: &Value) -> &str {
    payload(v)["type"]
        .as_str()
        .or_else(|| v["type"].as_str())
        .unwrap_or_default()
}

fn text_blocks(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("input_text" | "output_text" | "text") => {
                    b["text"].as_str().map(str::to_string)
                }
                Some("input_image" | "image") => Some("[image]".to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Codex wraps the environment, the project instructions and permission notes into user-role
/// messages of their own. They are context for the model, not something the person typed.
fn is_injected(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<user_instructions>")
        || t.starts_with("<INSTRUCTIONS>")
        || t.starts_with("<permissions")
        || t.starts_with("<user_action>")
        || t.starts_with("# AGENTS.md instructions")
}

fn when(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v["timestamp"].as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

fn note_meta(meta: &mut RolloutMeta, v: &Value) {
    let p = payload(v);
    match kind(v) {
        "session_meta" => {
            if meta.id.is_none() {
                meta.id = p["id"].as_str().map(str::to_string);
            }
            if meta.cwd.is_none() {
                meta.cwd = p["cwd"]
                    .as_str()
                    .filter(|c| !c.is_empty())
                    .map(str::to_string);
            }
        }
        "turn_context" => {
            if meta.cwd.is_none() {
                meta.cwd = p["cwd"]
                    .as_str()
                    .filter(|c| !c.is_empty())
                    .map(str::to_string);
            }
            if let Some(m) = p["model"].as_str().filter(|m| !m.is_empty()) {
                meta.model = Some(m.to_string());
            }
        }
        "message" if meta.first_prompt.is_none() && p["role"] == "user" => {
            let text = text_blocks(&p["content"]);
            if !text.trim().is_empty() && !is_injected(&text) {
                meta.first_prompt = Some(text);
            }
        }
        _ => {}
    }
}

/// The start of a rollout, enough to list it: id, directory, model and what was asked first.
pub(crate) fn head(path: &Path) -> RolloutMeta {
    use std::io::Read;
    let mut meta = RolloutMeta::default();
    let Ok(file) = std::fs::File::open(path) else {
        return meta;
    };
    let mut reader = BufReader::new(file.take(HEAD_BYTES));
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
            note_meta(&mut meta, &v);
        }
        line.clear();
        if meta.id.is_some()
            && meta.cwd.is_some()
            && meta.model.is_some()
            && meta.first_prompt.is_some()
        {
            break;
        }
    }
    if meta.id.is_none() {
        meta.id = id_from_filename(path);
    }
    meta
}

/// Text an assistant message gathered, and the calls it made, waiting for the results that follow.
#[derive(Default)]
struct Turn {
    text: String,
    calls: Vec<ToolCall>,
    model: Option<String>,
    at: Option<i64>,
}

impl Turn {
    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.calls.is_empty()
    }
}

fn parse_args(raw: &Value) -> Value {
    match raw {
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone())),
        other => other.clone(),
    }
}

/// A tool output is often `{"output": "...", "metadata": {...}}` serialised into a string.
fn output_text(raw: &Value) -> String {
    let inner = match raw {
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Object(_)) => v,
            _ => return s.clone(),
        },
        other => other.clone(),
    };
    match &inner {
        Value::Object(o) => o
            .get("output")
            .or_else(|| o.get("content"))
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => text_blocks(other),
            })
            .unwrap_or_else(|| inner.to_string()),
        other => text_blocks(other),
    }
}

/// Read a whole rollout into Forge messages: user and assistant text, tool calls on the assistant
/// message that made them, and their outputs as tool rows. Reasoning is encrypted and dropped.
pub(crate) fn read(path: &Path) -> Result<Rollout> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut meta = RolloutMeta::default();
    let mut out: Vec<Imported> = Vec::new();
    let mut turn = Turn::default();
    let mut current_model: Option<String> = None;

    let flush = |turn: &mut Turn, out: &mut Vec<Imported>| {
        if turn.is_empty() {
            *turn = Turn::default();
            return;
        }
        let t = std::mem::take(turn);
        out.push(Imported {
            role: Role::Assistant,
            content: t.text,
            model: t.model,
            tool_calls: t.calls,
            tool_call_id: None,
            live: true,
            timestamp: t.at,
        });
    };

    for raw in BufReader::new(file).lines() {
        let raw = raw?;
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        note_meta(&mut meta, &v);
        let p = payload(&v);
        match kind(&v) {
            "turn_context" => {
                current_model = p["model"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
            }
            "message" => {
                let body = text_blocks(&p["content"]);
                match p["role"].as_str() {
                    Some("user") => {
                        if body.trim().is_empty() || is_injected(&body) {
                            continue;
                        }
                        flush(&mut turn, &mut out);
                        out.push(Imported {
                            role: Role::User,
                            content: body,
                            model: None,
                            tool_calls: Vec::new(),
                            tool_call_id: None,
                            live: true,
                            timestamp: when(&v),
                        });
                    }
                    Some("assistant") => {
                        if body.trim().is_empty() {
                            continue;
                        }
                        // Text that arrives after calls were answered starts a new message.
                        if !turn.calls.is_empty() {
                            flush(&mut turn, &mut out);
                        }
                        if turn.is_empty() {
                            turn.model = current_model.clone();
                            turn.at = when(&v);
                        }
                        if !turn.text.is_empty() {
                            turn.text.push_str("\n\n");
                        }
                        turn.text.push_str(&body);
                    }
                    _ => {}
                }
            }
            "function_call" | "custom_tool_call" | "local_shell_call" => {
                let id = p["call_id"]
                    .as_str()
                    .or_else(|| p["id"].as_str())
                    .unwrap_or_default()
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let (name, args) = if kind(&v) == "local_shell_call" {
                    (
                        "shell".to_string(),
                        serde_json::json!({ "command": p["action"]["command"] }),
                    )
                } else {
                    (
                        p["name"].as_str().unwrap_or("tool").to_string(),
                        parse_args(if p.get("arguments").is_some() {
                            &p["arguments"]
                        } else {
                            &p["input"]
                        }),
                    )
                };
                if turn.is_empty() {
                    turn.model = current_model.clone();
                    turn.at = when(&v);
                }
                if !turn.calls.iter().any(|c| c.id == id) {
                    turn.calls.push(ToolCall { id, name, args });
                }
            }
            "function_call_output" | "custom_tool_call_output" => {
                let Some(id) = p["call_id"].as_str() else {
                    continue;
                };
                // The assistant message goes out before the first of its results.
                flush(&mut turn, &mut out);
                out.push(Imported {
                    role: Role::Tool,
                    content: output_text(&p["output"]),
                    model: None,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(id.to_string()),
                    live: true,
                    timestamp: when(&v),
                });
            }
            _ => {}
        }
    }
    flush(&mut turn, &mut out);
    if meta.id.is_none() {
        meta.id = id_from_filename(path);
    }
    // A model named by any turn is the one the next turn would use.
    meta.model = current_model.or(meta.model);

    let mut messages = Vec::with_capacity(out.len());
    let mut normalizer = Normalizer::default();
    let mut emit = |m: Imported| -> Result<()> {
        messages.push(m);
        Ok(())
    };
    for m in out {
        normalizer.push(m, &mut emit)?;
    }
    normalizer.finish(&mut emit)?;
    Ok(Rollout { meta, messages })
}

#[cfg(test)]
pub(crate) mod tests;
