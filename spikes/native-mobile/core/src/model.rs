use serde::Deserialize;

use crate::markdown::{self, MdBlock};

#[derive(Debug, Clone, Deserialize, uniffi::Record)]
pub struct SessionRow {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub waiting: bool,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub last_activity: i64,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub diff_added: Option<i64>,
    #[serde(default)]
    pub diff_removed: Option<i64>,
    #[serde(default)]
    pub last_turn_outcome: Option<String>,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub done_at: Option<i64>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum FleetEntry {
    Label { text: String },
    Session { row: SessionRow },
}

pub fn build_fleet(rows: Vec<SessionRow>) -> Vec<FleetEntry> {
    fn group(r: &SessionRow) -> u8 {
        if r.waiting { 0 } else if r.starred { 1 } else if r.busy { 2 } else { 3 }
    }
    let (done, mut live): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| r.done_at.is_some());
    live.sort_by_key(group);
    let mut out = Vec::new();
    let mut prev = u8::MAX;
    for r in live {
        let g = group(&r);
        if g != prev && g != 0 {
            let text = match g { 1 => "STARRED", 2 => "FORGING", _ => "COOL" };
            out.push(FleetEntry::Label { text: text.into() });
        }
        prev = g;
        out.push(FleetEntry::Session { row: r });
    }
    if !done.is_empty() {
        out.push(FleetEntry::Label { text: format!("DONE · {}", done.len()) });
    }
    out
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryRow {
    pub seq: i64,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub tool_phase: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Enum)]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChatItem {
    pub seq: i64,
    pub role: Role,
    pub tool: Option<String>,
    pub blocks: Vec<MdBlock>,
}

pub fn to_chat_item(r: HistoryRow) -> ChatItem {
    let role = match r.kind.as_deref().unwrap_or(&r.role) {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => Role::System,
    };
    let blocks = match role {
        Role::Assistant | Role::User => markdown::parse(&r.content),
        _ => vec![MdBlock::Code {
            lang: r.tool.clone().unwrap_or_default(),
            code: r.content.chars().take(600).collect(),
        }],
    };
    ChatItem { seq: r.seq, role, tool: r.tool, blocks }
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Enum)]
pub enum ConnectionState {
    Connecting,
    Open,
    Reconnecting,
    Closed,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SessionUpdate {
    pub revision: u64,
    pub title: String,
    pub busy: bool,
    pub activity: String,
    pub model: String,
    pub cost_usd: f64,
    pub streaming_blocks: Vec<MdBlock>,
    pub streaming_chars: u32,
}

/// Subset of the v10 snapshot the spike reads (full shape: `mobile/src/lib/ws.ts`).
#[derive(Debug, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub revision: Option<u64>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub activity: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub streaming: String,
    #[serde(default)]
    pub streaming_offset: Option<usize>,
    #[serde(default)]
    pub closed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, waiting: bool, busy: bool, starred: bool) -> SessionRow {
        SessionRow { id: id.into(), title: id.into(), cwd: String::new(), busy, waiting, cost_usd: 0.0, model: String::new(), last_activity: 0, branch: None, diff_added: None, diff_removed: None, last_turn_outcome: None, starred, done_at: None }
    }

    #[test]
    fn needs_you_first_without_label() {
        let f = build_fleet(vec![row("idle", false, false, false), row("busy", false, true, false), row("wait", true, false, false)]);
        assert!(matches!(&f[0], FleetEntry::Session { row } if row.id == "wait"));
        assert!(matches!(&f[1], FleetEntry::Label { text } if text == "FORGING"));
        assert!(matches!(&f[3], FleetEntry::Label { text } if text == "COOL"));
    }
}
