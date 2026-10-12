//! Forge session rows -> Claude Code transcript lines.
//!
//! Pure: rows in, JSON lines out. The writer ([`super::sync`]) decides where they go. What comes
//! out is what Forge's model would see next, laid out the way Claude Code writes it, so
//! `claude --resume` carries on from it.

use std::collections::HashMap;

use forge_store::ExportRow;
use forge_types::Role;
use serde_json::{json, Value};

const MISSING_RESULT: &str = "[no result recorded]";
const BLANK_RESULT: &str = "(no output)";
const SUMMARY_PREAMBLE: &str = "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.\n\nSummary:\n";
const LAST_PROMPT_CHARS: usize = 200;
/// What Claude Code records for a message that no real model wrote.
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Fields every entry Claude Code writes carries.
pub(crate) struct ExportContext<'a> {
    pub(crate) cc_session_id: &'a str,
    pub(crate) cwd: &'a str,
    pub(crate) version: &'a str,
    pub(crate) git_branch: Option<&'a str>,
}

#[derive(Debug, Default)]
pub(crate) struct Exported {
    /// One complete JSON line (newline included) per entry.
    pub(crate) lines: Vec<String>,
    /// The last chain entry written; the next append hangs off it.
    pub(crate) last_uuid: Option<String>,
}

impl Exported {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        self.lines.concat().into_bytes()
    }
}

/// A tool id Claude Code's API accepts (`^[A-Za-z0-9_-]+$`) that is also unique across turns.
/// Ids already well-formed and long enough to be unique (Claude's own `toolu_...`) are kept, so a
/// session imported from Claude Code comes back with the ids it left with; anything else is
/// rebuilt from the sanitized id and the seq of the assistant row that issued it.
pub(crate) fn cc_tool_id(id: &str, assistant_seq: i64) -> String {
    let clean = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if clean && id.len() >= 16 {
        return id.to_string();
    }
    let body: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    format!("toolu_fg{assistant_seq}_{body}")
}

fn exportable(row: &ExportRow) -> bool {
    row.active && !row.nudge && row.visibility.is_llm() && row.role != Role::System
}

fn model_name(model: Option<&str>) -> String {
    let bare = model
        .map(|m| m.rsplit("::").next().unwrap_or(m))
        .unwrap_or_default();
    if bare.starts_with("claude") {
        bare.to_string()
    } else {
        SYNTHETIC_MODEL.to_string()
    }
}

/// One message before it gets its uuid.
struct Draft {
    role: Role,
    blocks: Vec<Value>,
    ts_ms: i64,
    model: Option<String>,
}

impl Draft {
    fn user_text(&self) -> Option<String> {
        let texts: Vec<&str> = self
            .blocks
            .iter()
            .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten())
            .collect();
        (!texts.is_empty()).then(|| texts.join("\n\n"))
    }
}

fn push_draft(drafts: &mut Vec<Draft>, d: Draft) {
    if let Some(last) = drafts.last_mut() {
        if last.role == d.role {
            // Two same-role entries in a row would be one turn to the API; make them one entry.
            let merged_text = match (last.blocks.last_mut(), d.blocks.first()) {
                (Some(a), Some(b)) if a["type"] == "text" && b["type"] == "text" => {
                    let joined = format!(
                        "{}\n\n{}",
                        a["text"].as_str().unwrap_or_default(),
                        b["text"].as_str().unwrap_or_default()
                    );
                    a["text"] = Value::String(joined);
                    true
                }
                _ => false,
            };
            last.blocks
                .extend(d.blocks.into_iter().skip(usize::from(merged_text)));
            last.ts_ms = last.ts_ms.max(d.ts_ms);
            if d.model.is_some() {
                last.model = d.model;
            }
            return;
        }
    }
    drafts.push(d);
}

fn drafts_from_rows(rows: &[ExportRow]) -> Vec<Draft> {
    let mut drafts: Vec<Draft> = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let row = &rows[i];
        i += 1;
        if !exportable(row) {
            continue;
        }
        let ts_ms = row.created_at * 1000;
        match row.role {
            Role::User => {
                if !row.content.trim().is_empty() {
                    push_draft(
                        &mut drafts,
                        Draft {
                            role: Role::User,
                            blocks: vec![json!({"type": "text", "text": row.content})],
                            ts_ms,
                            model: None,
                        },
                    );
                }
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !row.content.trim().is_empty() {
                    blocks.push(json!({"type": "text", "text": row.content}));
                }
                let mut ids: Vec<(String, String)> = Vec::new();
                for call in &row.tool_calls {
                    let id = cc_tool_id(&call.id, row.seq);
                    let (name, input) = forge_core::forge_tool_to_cc(&call.name, &call.args);
                    let input = if input.is_object() { input } else { json!({}) };
                    blocks
                        .push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
                    ids.push((call.id.clone(), id));
                }
                // The results that follow, keyed by the id Forge stored them under.
                let mut results: HashMap<&str, &str> = HashMap::new();
                let mut last_result_ts = ts_ms;
                while let Some(next) = rows.get(i).filter(|r| r.role == Role::Tool) {
                    i += 1;
                    if let (true, Some(id)) = (exportable(next), next.tool_call_id.as_deref()) {
                        results.insert(id, next.content.as_str());
                        last_result_ts = last_result_ts.max(next.created_at * 1000);
                    }
                }
                if blocks.is_empty() {
                    continue;
                }
                push_draft(
                    &mut drafts,
                    Draft {
                        role: Role::Assistant,
                        blocks,
                        ts_ms,
                        model: Some(model_name(row.model.as_deref())),
                    },
                );
                if !ids.is_empty() {
                    let blocks = ids
                        .iter()
                        .map(|(orig, id)| {
                            let text = results
                                .get(orig.as_str())
                                .copied()
                                .unwrap_or(MISSING_RESULT);
                            let text = if text.trim().is_empty() {
                                BLANK_RESULT
                            } else {
                                text
                            };
                            json!({"type": "tool_result", "tool_use_id": id, "content": text})
                        })
                        .collect();
                    push_draft(
                        &mut drafts,
                        Draft {
                            role: Role::User,
                            blocks,
                            ts_ms: last_result_ts,
                            model: None,
                        },
                    );
                }
            }
            // A result whose call is not directly before it answers nothing.
            Role::Tool | Role::System => {}
        }
    }
    drafts
}

fn rfc3339(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

struct Writer<'a> {
    ctx: &'a ExportContext<'a>,
    out: Exported,
    parent: Option<String>,
    clock_ms: i64,
}

impl Writer<'_> {
    fn stamp(&mut self, wanted_ms: i64) -> String {
        self.clock_ms = wanted_ms.max(self.clock_ms + 1);
        rfc3339(self.clock_ms)
    }

    fn base(
        &mut self,
        kind: &str,
        ts_ms: i64,
        parent: Option<&str>,
    ) -> (serde_json::Map<String, Value>, String) {
        let uuid = forge_types::new_id();
        let ts = self.stamp(ts_ms);
        let mut m = serde_json::Map::new();
        m.insert(
            "parentUuid".into(),
            parent.map_or(Value::Null, |p| json!(p)),
        );
        m.insert("isSidechain".into(), json!(false));
        m.insert("type".into(), json!(kind));
        m.insert("uuid".into(), json!(uuid));
        m.insert("timestamp".into(), json!(ts));
        m.insert("userType".into(), json!("external"));
        m.insert("entrypoint".into(), json!("cli"));
        m.insert("cwd".into(), json!(self.ctx.cwd));
        m.insert("sessionId".into(), json!(self.ctx.cc_session_id));
        m.insert("version".into(), json!(self.ctx.version));
        if let Some(b) = self.ctx.git_branch {
            m.insert("gitBranch".into(), json!(b));
        }
        (m, uuid)
    }

    fn finish(&mut self, m: serde_json::Map<String, Value>, uuid: String) {
        self.out.lines.push(format!("{}\n", Value::Object(m)));
        self.parent = Some(uuid.clone());
        self.out.last_uuid = Some(uuid);
    }

    fn meta(&mut self, v: Value) {
        self.out.lines.push(format!("{v}\n"));
    }

    fn boundary(&mut self, summary: &str, ts_ms: i64) {
        let logical = self.parent.clone();
        let (mut m, uuid) = self.base("system", ts_ms, None);
        m.insert("subtype".into(), json!("compact_boundary"));
        m.insert("content".into(), json!("Conversation compacted"));
        m.insert("isMeta".into(), json!(false));
        m.insert("level".into(), json!("info"));
        if let Some(l) = logical {
            m.insert("logicalParentUuid".into(), json!(l));
        }
        m.insert(
            "compactMetadata".into(),
            json!({"trigger": "manual", "preTokens": 0}),
        );
        self.finish(m, uuid.clone());
        let text = if summary
            .trim_start()
            .starts_with("This session is being continued")
        {
            summary.to_string()
        } else {
            format!("{SUMMARY_PREAMBLE}{}", summary.trim())
        };
        let (mut m, uuid) = self.base("user", ts_ms, Some(&uuid));
        m.insert("message".into(), json!({"role": "user", "content": text}));
        m.insert("isCompactSummary".into(), json!(true));
        m.insert("isVisibleInTranscriptOnly".into(), json!(true));
        self.finish(m, uuid);
    }

    fn draft(&mut self, d: Draft) {
        let parent = self.parent.clone();
        let (mut m, uuid) = self.base(
            if d.role == Role::Assistant {
                "assistant"
            } else {
                "user"
            },
            d.ts_ms,
            parent.as_deref(),
        );
        if d.role == Role::Assistant {
            let calls = d.blocks.iter().any(|b| b["type"] == "tool_use");
            m.insert(
                "message".into(),
                json!({
                    "model": d.model.unwrap_or_else(|| SYNTHETIC_MODEL.into()),
                    "id": format!("msg_{}", &uuid.replace('-', "")[..24]),
                    "type": "message",
                    "role": "assistant",
                    "content": d.blocks,
                    "stop_reason": if calls { "tool_use" } else { "end_turn" },
                    "stop_sequence": null,
                    "usage": {"input_tokens": 0, "output_tokens": 0,
                              "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
                }),
            );
        } else {
            let content = match d.user_text() {
                Some(t) if d.blocks.len() == 1 => json!(t),
                _ => json!(d.blocks),
            };
            m.insert(
                "message".into(),
                json!({"role": "user", "content": content}),
            );
        }
        self.finish(m, uuid);
    }
}

/// Lay `rows` out as transcript lines hanging off `parent` (`None` starts a fresh file).
///
/// `title` adds a `custom-title` entry (first export only). `summary` starts with a compaction
/// boundary and the summary entry, after which `rows` are the live rows.
pub(crate) fn build_entries(
    ctx: &ExportContext<'_>,
    parent: Option<&str>,
    title: Option<&str>,
    summary: Option<&str>,
    rows: &[ExportRow],
) -> Exported {
    let drafts = drafts_from_rows(rows);
    let first_ms = drafts
        .first()
        .map(|d| d.ts_ms)
        .or_else(|| rows.first().map(|r| r.created_at * 1000))
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let mut w = Writer {
        ctx,
        out: Exported::default(),
        parent: parent.map(str::to_string),
        clock_ms: 0,
    };
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        w.meta(json!({"type": "custom-title", "customTitle": t, "sessionId": ctx.cc_session_id}));
    }
    if let Some(s) = summary.filter(|s| !s.trim().is_empty()) {
        w.boundary(s, first_ms);
    }
    let last_prompt = drafts
        .iter()
        .rev()
        .find(|d| d.role == Role::User && !d.blocks.iter().any(|b| b["type"] == "tool_result"))
        .and_then(Draft::user_text);
    for d in drafts {
        w.draft(d);
    }
    if let (Some(prompt), Some(leaf)) = (last_prompt, w.out.last_uuid.clone()) {
        let prompt: String = prompt.chars().take(LAST_PROMPT_CHARS).collect();
        w.meta(json!({"type": "last-prompt", "lastPrompt": prompt, "leafUuid": leaf, "sessionId": ctx.cc_session_id}));
    }
    w.out
}
