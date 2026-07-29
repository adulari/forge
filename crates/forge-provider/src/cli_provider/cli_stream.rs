//! CLI bridge stream parsing and quota event normalization.

use super::*;

impl ClaudeStreamState {
    pub(super) fn parse_line(&mut self, line: &str) -> Vec<Parsed> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        match v.get("type").and_then(Value::as_str) {
            Some("stream_event") => {
                let Some(event) = v.get("event") else {
                    return out;
                };
                match event.get("type").and_then(Value::as_str) {
                    Some("message_start") => {
                        self.text.clear();
                        self.reasoning.clear();
                        self.tool_ids.clear();
                        self.stream_message_active = true;
                        return out;
                    }
                    Some("message_stop") => {
                        self.stream_message_active = false;
                        return out;
                    }
                    Some("content_block_delta") => {}
                    _ => return out,
                }
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let Some(delta) = event.get("delta") else {
                    return out;
                };
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                self.text.entry(index).or_default().push_str(text);
                                out.push(Parsed::Text(text.to_string()));
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(thinking) = delta.get("thinking").and_then(Value::as_str) {
                            if !thinking.is_empty() {
                                self.reasoning.entry(index).or_default().push_str(thinking);
                                out.push(Parsed::Reasoning(thinking.to_string()));
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Consolidated assistant message: emit only bytes that were not already streamed.
            Some("assistant") => {
                let belongs_to_stream_message = self.stream_message_active;
                if let Some(blocks) = v
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_array)
                {
                    for (index, block) in blocks.iter().enumerate() {
                        match block.get("type").and_then(Value::as_str) {
                            Some("thinking") => {
                                if let Some(full) = block
                                    .get("thinking")
                                    .and_then(Value::as_str)
                                    .filter(|s| !s.is_empty())
                                {
                                    if let Some(unseen) =
                                        unseen_consolidated_suffix(full, &mut self.reasoning, index)
                                    {
                                        out.push(Parsed::Reasoning(unseen.to_string()));
                                    }
                                }
                            }
                            Some("text") => {
                                if let Some(full) = block
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .filter(|s| !s.is_empty())
                                {
                                    if let Some(unseen) =
                                        unseen_consolidated_suffix(full, &mut self.text, index)
                                    {
                                        out.push(Parsed::Text(unseen.to_string()));
                                    }
                                }
                            }
                            Some("tool_use") => {
                                let id = block
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string();
                                let name = block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("tool")
                                    .to_string();
                                let args = block
                                    .get("input")
                                    .map(ToString::to_string)
                                    .unwrap_or_default();
                                if belongs_to_stream_message
                                    && !id.is_empty()
                                    && !self.tool_ids.insert(id.clone())
                                {
                                    continue;
                                }
                                out.push(Parsed::ToolStarted { id, name, args });
                            }
                            _ => {}
                        }
                    }
                }
                // With partial streaming, Claude emits one block-level assistant snapshot before
                // the message's later blocks and `message_stop`; retain state so a duplicate
                // snapshot cannot repeat streamed bytes. A non-streamed standalone assistant event
                // has no trustworthy message boundary, so keep the conservative legacy reset.
                if !belongs_to_stream_message {
                    self.text.clear();
                    self.reasoning.clear();
                    self.tool_ids.clear();
                }
            }
            // User message: tool_result blocks (the outcome of a tool the agent ran).
            Some("user") => {
                if let Some(blocks) = v
                    .get("message")
                    .and_then(|message| message.get("content"))
                    .and_then(Value::as_array)
                {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                            let id = block
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let ok = !block
                                .get("is_error")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            let summary = tool_result_summary(block.get("content"));
                            out.push(Parsed::ToolFinished { id, ok, summary });
                        }
                    }
                }
            }
            Some("result") => {
                self.stream_message_active = false;
                self.text.clear();
                self.reasoning.clear();
                self.tool_ids.clear();
                if let Some(usage) = v.get("usage").map(usage_from) {
                    out.push(Parsed::Usage(usage));
                }
                let result_text = v.get("result").and_then(Value::as_str).map(str::to_string);
                if let Some(final_text) = &result_text {
                    out.push(Parsed::Final(final_text.clone()));
                }
                if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                    out.push(Parsed::Error(
                        v.get("api_error_status")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or(result_text)
                            .unwrap_or_else(|| truncated_event_json(&v)),
                    ));
                }
            }
            // Subscription quota window (Claude Code stream-json, L3). Defensive: any missing field
            // degrades to Ok / None. `resetsAt` may arrive as secs or ms — normalise ms→secs.
            Some("rate_limit_event") => {
                if let Some(info) = v.get("rate_limit_info") {
                    let status = info.get("status").and_then(Value::as_str).unwrap_or("");
                    let using_overage = info
                        .get("isUsingOverage")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let fraction = info
                        .get("utilization")
                        .or_else(|| info.get("usedFraction"))
                        .or_else(|| info.get("fractionUsed"))
                        .and_then(Value::as_f64);
                    let resets_at = info.get("resetsAt").and_then(Value::as_i64).map(|time| {
                        if time > 100_000_000_000 {
                            time / 1000
                        } else {
                            time
                        }
                    });
                    out.push(Parsed::Quota {
                        window: normalize_window(
                            info.get("rateLimitType")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                        ),
                        status: quota_status_from(status, using_overage, fraction),
                        resets_at,
                        fraction,
                    });
                }
            }
            _ => {}
        }
        out
    }
}

fn unseen_consolidated_suffix<'a>(
    full: &'a str,
    streamed: &mut std::collections::HashMap<usize, String>,
    consolidated_index: usize,
) -> Option<&'a str> {
    if let Some(preferred) = streamed
        .get(&consolidated_index)
        .filter(|text| !text.is_empty())
        .cloned()
    {
        let unseen = match full.strip_prefix(preferred.as_str()) {
            Some("") => None,
            Some(suffix) => Some(suffix),
            None => {
                // The protocol promises that consolidated blocks assemble the emitted deltas. If a
                // future CLI violates that, suppressing the repeated block is safer than duplicating
                // a whole answer in the UI. The authoritative `result.result` remains available.
                tracing::debug!("Claude consolidated block diverged from its streamed prefix");
                None
            }
        };
        streamed.insert(consolidated_index, full.to_string());
        return unseen;
    }

    // Claude Code 2.1.220 emits block-level `assistant` snapshots: `message.content` contains only
    // the just-finished block at array position zero, while the preceding `stream_event` retains
    // that block's original message index (for example text index 1 after thinking index 0).
    // Match the streamed value itself when the array position is therefore not an identity. Choose
    // the longest prefix so a shorter earlier block cannot steal a later block's suffix.
    let matching = streamed
        .iter()
        .filter(|(_, text)| !text.is_empty() && full.starts_with(text.as_str()))
        .max_by_key(|(_, text)| text.len())
        .map(|(index, text)| (*index, text.clone()));
    match matching {
        Some((index, text)) => {
            let unseen = match full.strip_prefix(text.as_str()) {
                Some("") => None,
                suffix => suffix,
            };
            streamed.insert(index, full.to_string());
            unseen
        }
        None => {
            streamed.insert(consolidated_index, full.to_string());
            Some(full)
        }
    }
}

/// Parse one Claude line in isolation. Runtime paths retain a [`ClaudeStreamState`] across lines;
/// this wrapper preserves deterministic single-line parsing for protocol/unit tests.
pub(super) fn parse_claude_line(line: &str) -> Vec<Parsed> {
    ClaudeStreamState::default().parse_line(line)
}

/// Collapse a tool_result `content` (string, or array of {type:text,text}) into a short summary.
fn tool_result_summary(content: Option<&Value>) -> String {
    let text = match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    let one_line = text.split('\n').next().unwrap_or("").trim();
    one_line.chars().take(120).collect()
}

/// Fallback detail for a CLI error event that carries no recognizable `message` field: the whole
/// event JSON, truncated. A generic "codex reported an error" hid the actual cause (quota, plan
/// restrictions, bad flags) — the raw event is the only diagnostic there is.
pub(super) fn truncated_event_json(v: &Value) -> String {
    const MAX_CHARS: usize = 500;
    let raw = v.to_string();
    if raw.chars().count() <= MAX_CHARS {
        raw
    } else {
        let head: String = raw.chars().take(MAX_CHARS).collect();
        format!("{head}…")
    }
}

fn codex_item_id(item: Option<&Value>) -> String {
    item.and_then(|i| i.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn codex_item_text(item: Option<&Value>) -> Option<String> {
    item.and_then(|i| i.get("text"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn codex_tool_started(item: Option<&Value>) -> Parsed {
    Parsed::ToolStarted {
        id: codex_item_id(item),
        name: item
            .and_then(|i| i.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string(),
        args: item
            .and_then(|i| i.get("arguments"))
            .map(|a| a.to_string())
            .unwrap_or_default(),
    }
}

fn codex_command_started(item: Option<&Value>) -> Parsed {
    Parsed::ToolStarted {
        id: codex_item_id(item),
        name: "shell".to_string(),
        args: item
            .and_then(|i| i.get("command"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// Parse one Codex `exec --json` (JSONL) line into zero or more items. Handles agent text,
/// reasoning summaries, `mcp_tool_call` (Forge's tools) and `command_execution` (codex's own
/// read-only shell). Field-tolerant: unknown shapes are ignored so CLI drift degrades gracefully.
pub(super) fn parse_codex_line(line: &str) -> Vec<Parsed> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match v.get("type").and_then(Value::as_str) {
        // A tool call begins (mcp__forge tool or codex's own read-only shell).
        Some("item.started") => {
            let item = v.get("item");
            match item.and_then(|i| i.get("type")).and_then(Value::as_str) {
                Some("mcp_tool_call") => vec![codex_tool_started(item)],
                Some("command_execution") => vec![codex_command_started(item)],
                _ => Vec::new(),
            }
        }
        Some("item.completed") => {
            let item = v.get("item");
            let kind = item.and_then(|i| i.get("type")).and_then(Value::as_str);
            let id = || codex_item_id(item);
            match kind {
                Some("agent_message") => codex_item_text(item)
                    .map(Parsed::Text)
                    .into_iter()
                    .collect(),
                Some("reasoning") => codex_item_text(item)
                    .map(Parsed::Reasoning)
                    .into_iter()
                    .collect(),
                Some("mcp_tool_call") => {
                    let ok = item.and_then(|i| i.get("status")).and_then(Value::as_str)
                        != Some("failed");
                    let summary = item
                        .and_then(|i| i.get("error"))
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            item.and_then(|i| i.get("tool"))
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    vec![Parsed::ToolFinished {
                        id: id(),
                        ok,
                        summary,
                    }]
                }
                Some("command_execution") => {
                    let ok = item
                        .and_then(|i| i.get("exit_code"))
                        .and_then(Value::as_i64)
                        == Some(0);
                    let summary = item
                        .and_then(|i| i.get("command"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .chars()
                        .take(120)
                        .collect();
                    vec![Parsed::ToolFinished {
                        id: id(),
                        ok,
                        summary,
                    }]
                }
                _ => Vec::new(),
            }
        }
        // The session id — its rollout file (~/.codex/sessions/.../rollout-*-<id>.jsonl) carries
        // the `token_count.rate_limits` snapshot the TUI's usage bar shows, which `exec --json`
        // omits from stdout. `complete` reads it post-turn for quota-aware routing (L3).
        Some("thread.started") => v
            .get("thread_id")
            .and_then(Value::as_str)
            .map(|id| vec![Parsed::Thread(id.to_string())])
            .unwrap_or_default(),
        Some("turn.completed") => v
            .get("usage")
            .map(usage_from)
            .map(Parsed::Usage)
            .into_iter()
            .collect(),
        Some(t) if t.contains("error") || t.contains("failed") => vec![Parsed::Error(
            v.get("message")
                .and_then(Value::as_str)
                .or_else(|| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                })
                .map(str::to_string)
                .unwrap_or_else(|| truncated_event_json(&v)),
        )],
        _ => Vec::new(),
    }
}

pub(super) fn parse_line(kind: CliKind, line: &str) -> Vec<Parsed> {
    match kind {
        CliKind::ClaudeCode => parse_claude_line(line),
        CliKind::Codex => parse_codex_line(line),
        CliKind::Antigravity => parse_antigravity_line(line),
    }
}

pub(super) fn parse_stream_line(
    kind: CliKind,
    line: &str,
    claude: &mut ClaudeStreamState,
) -> Vec<Parsed> {
    match kind {
        CliKind::ClaudeCode => claude.parse_line(line),
        CliKind::Codex | CliKind::Antigravity => parse_line(kind, line),
    }
}

/// agy `-p` prints the answer as PLAIN TEXT (no JSON event stream like claude/codex), so every
/// non-empty stdout line is answer text that accumulates into the final response. There are no
/// tool/usage/quota events to parse — usage stays $0 (free Gemini tier) and the answer is the
/// accumulated text.
pub(super) fn parse_antigravity_line(line: &str) -> Vec<Parsed> {
    if line.trim().is_empty() {
        Vec::new()
    } else {
        vec![Parsed::Text(format!("{line}\n"))]
    }
}
