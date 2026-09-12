//! What the board derives from the daemon's data: the column a session belongs in, its health,
//! the signals worth a glance, and the small formatting helpers cards use.
//!
//! Everything here is pure and testable without a terminal or a daemon.

use std::collections::HashMap;

pub use crate::board::wire::*;

/// The board's columns, left to right. Sessions that need a human come first, exactly like the
/// fleet ordering every other Forge surface uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Column {
    /// Blocked on a permission/question, stalled, or stopped badly — a human should look.
    Attention,
    /// A turn is running.
    Working,
    /// Idle and healthy: finished its last turn well, or never started one.
    Ready,
    /// Not running any more (persisted sessions, resumable).
    Done,
}

impl Column {
    pub const ALL: [Column; 4] = [
        Column::Attention,
        Column::Working,
        Column::Ready,
        Column::Done,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Attention => "Needs you",
            Self::Working => "Working",
            Self::Ready => "Ready",
            Self::Done => "Done",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }
}

/// The one-word health verdict shown on the card's status dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// A permission prompt or question is pending.
    Waiting,
    /// Busy but showing loop/stall symptoms, or silent for a long time.
    Stalled,
    /// Last turn stopped badly (no output, step/budget cap, interrupted) and nothing has run since.
    Failed,
    Busy,
    Idle,
    /// A past session.
    Finished,
}

impl Health {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "needs decision",
            Self::Stalled => "stalled",
            Self::Failed => "stopped badly",
            Self::Busy => "working",
            Self::Idle => "idle",
            Self::Finished => "finished",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignalLevel {
    Info,
    Warn,
    Danger,
}

/// One glanceable fact about a session that the user would otherwise have to dig for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    pub level: SignalLevel,
    pub text: String,
}

impl Signal {
    fn danger(text: impl Into<String>) -> Self {
        Self {
            level: SignalLevel::Danger,
            text: text.into(),
        }
    }
    fn warn(text: impl Into<String>) -> Self {
        Self {
            level: SignalLevel::Warn,
            text: text.into(),
        }
    }
    fn info(text: impl Into<String>) -> Self {
        Self {
            level: SignalLevel::Info,
            text: text.into(),
        }
    }
}

/// Busy with no activity for this long reads as "quiet" — worth a look, not yet an alarm.
pub const QUIET_AFTER_SECS: i64 = 180;
/// Busy with no activity for this long is treated as stalled.
pub const STALL_AFTER_SECS: i64 = 600;
/// Context fill at which the card warns.
pub const CONTEXT_WARN_PCT: u8 = 80;
/// How many consecutive assistant rows must open with the same sentence to count as repeating.
pub const REPEAT_OPENING_ROWS: usize = 3;

/// One card on the board — a live session (fleet row + its latest snapshot) or a past one.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub worktree: Option<String>,
    pub model: String,
    pub tier: Option<String>,
    pub column: Column,
    pub health: Health,
    pub signals: Vec<Signal>,
    /// The in-progress task, else the first pending one.
    pub current_task: Option<String>,
    pub tasks_done: usize,
    pub tasks_total: usize,
    /// The newest transcript line, or the streaming edge while a reply is in flight.
    pub last_line: Option<String>,
    pub streaming: bool,
    pub cost_usd: f64,
    pub context_pct: Option<u8>,
    pub last_activity: i64,
    pub created_at: i64,
    pub subagents: Vec<Subagent>,
    pub queued: usize,
    pub busy: bool,
    pub waiting: bool,
    pub read_only: bool,
    pub terminal: bool,
    pub past: bool,
    pub archived: bool,
    pub message_count: i64,
}

impl Card {
    pub fn short_id(&self) -> String {
        self.id.chars().take(8).collect()
    }

    /// What the card is called: title, else the id prefix.
    pub fn display_title(&self) -> String {
        if self.title.trim().is_empty() {
            format!("session {}", self.short_id())
        } else {
            self.title.clone()
        }
    }

    pub fn project(&self) -> String {
        project_name(&self.cwd)
    }
}

/// Build a live card from the fleet row and (when the board has one) the session's snapshot.
pub fn live_card(row: &FleetRow, snap: Option<&LiveSnapshot>, now: i64) -> Card {
    let busy = snap.map_or(row.busy, |s| s.busy);
    let waiting = snap.map_or(row.waiting, LiveSnapshot::waiting);
    let outcome = snap
        .and_then(|s| s.last_turn_outcome.clone())
        .or_else(|| row.last_turn_outcome.clone());
    let stop = snap
        .and_then(|s| s.last_stop_reason.clone())
        .or_else(|| row.last_stop_reason.clone());
    let signals = live_signals(row, snap, now);
    let stalled = busy && signals.iter().any(|s| s.level == SignalLevel::Danger);
    let failed = !busy && !waiting && outcome.as_deref() == Some("failed");
    let health = if waiting {
        Health::Waiting
    } else if stalled {
        Health::Stalled
    } else if failed {
        Health::Failed
    } else if busy {
        Health::Busy
    } else {
        Health::Idle
    };
    let column = match health {
        Health::Waiting | Health::Stalled | Health::Failed => Column::Attention,
        Health::Busy => Column::Working,
        Health::Idle | Health::Finished => Column::Ready,
    };
    let tasks = snap.map(|s| s.tasks.as_slice()).unwrap_or(&[]);
    let current_task = tasks
        .iter()
        .find(|t| t.status == "in_progress")
        .or_else(|| tasks.iter().find(|t| t.status == "pending"))
        .map(|t| t.title.clone());
    let tasks_done = tasks.iter().filter(|t| t.status == "done").count();
    let (last_line, streaming) = match snap {
        Some(s) if !s.streaming.trim().is_empty() => (Some(last_sentence(&s.streaming)), true),
        Some(s) => (
            s.rows()
                .iter()
                .rev()
                .find(|r| r.kind != "system" && !r.text.trim().is_empty())
                .map(|r| r.text.clone()),
            false,
        ),
        None => (None, false),
    };
    let cost = snap.map_or(row.cost_usd, |s| s.cost_usd);
    let ctx_tokens = snap.map_or(row.context_tokens, |s| s.context_tokens);
    let ctx_limit = snap.and_then(|s| s.context_limit).or(row.context_limit);
    let _ = stop;
    Card {
        id: row.id.clone(),
        title: snap
            .map(|s| s.title.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| row.title.clone()),
        cwd: row.cwd.clone(),
        worktree: row.worktree.clone(),
        model: snap.map_or(row.model.clone(), |s| s.model.clone()),
        tier: snap.and_then(|s| s.tier.clone()),
        column,
        health,
        signals,
        current_task,
        tasks_done,
        tasks_total: tasks.len(),
        last_line,
        streaming,
        cost_usd: cost,
        context_pct: context_pct(ctx_tokens, ctx_limit),
        last_activity: row.last_activity,
        created_at: row.created_at,
        subagents: snap.map(|s| s.subagents.clone()).unwrap_or_default(),
        queued: snap.map_or(0, |s| s.queued.len()),
        busy,
        waiting,
        read_only: row.read_only,
        terminal: row.terminal,
        past: false,
        archived: false,
        message_count: 0,
    }
}

/// Build a card for a persisted (not running) session.
pub fn past_card(row: &PastRow) -> Card {
    let title = if row.title.trim().is_empty() {
        row.preview
            .as_deref()
            .map(|p| forge_types::truncate_ellipsis(p.lines().next().unwrap_or(""), 60))
            .unwrap_or_default()
    } else {
        row.title.clone()
    };
    Card {
        id: row.id.clone(),
        title,
        cwd: row.cwd.clone(),
        worktree: row.worktree.clone(),
        model: String::new(),
        tier: None,
        column: Column::Done,
        health: Health::Finished,
        signals: if row.archived {
            vec![Signal::info("archived")]
        } else {
            Vec::new()
        },
        current_task: None,
        tasks_done: 0,
        tasks_total: 0,
        last_line: row.preview.clone(),
        streaming: false,
        cost_usd: row.cost_usd,
        context_pct: None,
        last_activity: row.last_activity,
        created_at: row.created_at,
        subagents: Vec::new(),
        queued: 0,
        busy: false,
        waiting: false,
        read_only: true,
        terminal: false,
        past: true,
        archived: row.archived,
        message_count: row.message_count,
    }
}

/// The signals for a live session, most severe first. Every one of these is something a person
/// had to dig out of the journal or the store by hand at least once.
pub fn live_signals(row: &FleetRow, snap: Option<&LiveSnapshot>, now: i64) -> Vec<Signal> {
    let mut out = Vec::new();
    let busy = snap.map_or(row.busy, |s| s.busy);
    let waiting = snap.map_or(row.waiting, LiveSnapshot::waiting);
    if waiting {
        let what = snap
            .and_then(|s| {
                s.permission_prompt
                    .as_ref()
                    .map(|_| "permission")
                    .or_else(|| s.question.as_ref().map(|_| "question"))
            })
            .unwrap_or("decision");
        out.push(Signal::danger(format!("waiting on a {what}")));
    }
    if busy && !waiting {
        let quiet = now.saturating_sub(row.last_activity);
        if quiet >= STALL_AFTER_SECS {
            out.push(Signal::danger(format!("silent for {}", fmt_age(quiet))));
        } else if quiet >= QUIET_AFTER_SECS {
            out.push(Signal::warn(format!("quiet for {}", fmt_age(quiet))));
        }
    }
    if let Some(s) = snap {
        let rows = s.rows();
        if let Some(n) = repeated_opening(&rows) {
            out.push(Signal::danger(format!("repeating the same opening ×{n}")));
        }
        for r in rows.iter().rev().take(40) {
            if r.kind != "system" {
                continue;
            }
            let t = r.text.to_ascii_lowercase();
            if t.contains("benched") && t.contains("pinned") {
                out.push(Signal::danger("pinned model is benched"));
                break;
            }
            if t.contains("empty response") {
                out.push(Signal::danger("model returned empty responses"));
                break;
            }
            if t.contains("same sentence") || t.contains("stopping to avoid a loop") {
                out.push(Signal::danger("stall guard fired"));
                break;
            }
            if t.contains("not moved for") || t.contains("stalled task") {
                out.push(Signal::warn("a task has stalled"));
                break;
            }
        }
        if !busy && !waiting && s.last_turn_outcome.as_deref() == Some("failed") {
            out.push(Signal::warn(format!(
                "stopped: {}",
                stop_reason_words(s.last_stop_reason.as_deref())
            )));
        }
        let failed_tools = rows
            .iter()
            .rev()
            .take(12)
            .filter(|r| r.kind == "tool" && r.meta.as_deref() == Some("failed"))
            .count();
        if failed_tools >= 3 {
            out.push(Signal::warn(format!("{failed_tools} recent tool failures")));
        }
        if !s.queued.is_empty() {
            out.push(Signal::info(format!("{} queued", s.queued.len())));
        }
        if s.workflow.as_ref().is_some_and(|w| w.active) {
            out.push(Signal::info("workflow running"));
        }
        if s.plan.is_some() && waiting {
            out.push(Signal::info("plan awaiting approval"));
        }
    } else if !busy && !waiting && row.last_turn_outcome.as_deref() == Some("failed") {
        out.push(Signal::warn(format!(
            "stopped: {}",
            stop_reason_words(row.last_stop_reason.as_deref())
        )));
    }
    let ctx_tokens = snap.map_or(row.context_tokens, |s| s.context_tokens);
    let ctx_limit = snap.and_then(|s| s.context_limit).or(row.context_limit);
    if let Some(pct) = context_pct(ctx_tokens, ctx_limit) {
        if pct >= CONTEXT_WARN_PCT {
            out.push(Signal::warn(format!("context {pct}% full")));
        }
    }
    if row.read_only {
        out.push(Signal::info("read-only (no input path)"));
    } else if row.terminal {
        out.push(Signal::info("runs in a terminal"));
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.level));
    out.dedup();
    out
}

/// `Some(n)` when the last `n >= REPEAT_OPENING_ROWS` assistant rows open with the same sentence.
pub fn repeated_opening(rows: &[TranscriptRow]) -> Option<usize> {
    let openings: Vec<String> = rows
        .iter()
        .filter(|r| r.kind == "assistant" && r.text.trim().len() >= 24)
        .map(|r| first_sentence(&r.text))
        .collect();
    let last = openings.last()?;
    let n = openings.iter().rev().take_while(|o| *o == last).count();
    (n >= REPEAT_OPENING_ROWS).then_some(n)
}

fn first_sentence(text: &str) -> String {
    let t = text.trim();
    let end = t
        .find(['.', '!', '?', '\n'])
        .map_or(t.len(), |i| i + 1)
        .min(120);
    let end = t
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= end)
        .last()
        .unwrap_or(0);
    t[..end].trim().to_ascii_lowercase()
}

fn last_sentence(text: &str) -> String {
    let t = text.trim();
    let tail: String = t
        .chars()
        .rev()
        .take(160)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    tail.replace('\n', " ")
}

pub fn context_pct(tokens: u64, limit: Option<u32>) -> Option<u8> {
    let limit = limit.filter(|l| *l > 0)? as u64;
    Some(((tokens * 100) / limit).min(100) as u8)
}

/// `StopReason` on the wire → a phrase a person reads.
pub fn stop_reason_words(reason: Option<&str>) -> String {
    match reason {
        Some("no_output") => "no output".into(),
        Some("max_steps") => "hit the step cap".into(),
        Some("budget_exhausted") => "budget exhausted".into(),
        Some("interrupted") => "interrupted".into(),
        Some("final_answer") => "answered".into(),
        Some(other) => other.replace('_', " "),
        None => "failed".into(),
    }
}

/// `1h 12m` / `43s` / `3d` — coarse, for a card.
pub fn fmt_age(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        let h = s / 3600;
        let m = (s % 3600) / 60;
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m}m")
        }
    } else {
        format!("{}d", s / 86_400)
    }
}

pub fn fmt_cost(usd: f64) -> String {
    if usd == 0.0 {
        "$0".into()
    } else if usd < 0.01 {
        format!("${usd:.4}")
    } else if usd < 10.0 {
        format!("${usd:.2}")
    } else {
        format!("${usd:.1}")
    }
}

/// `provider::model` → `model`; a bare bridge id (`claude-cli::`) → its provider name.
pub fn model_short(id: &str) -> String {
    match id.split_once("::") {
        Some((provider, "")) => provider.to_string(),
        Some((_, model)) => model.to_string(),
        None if id.is_empty() || id == "—" => "no model yet".into(),
        None => id.to_string(),
    }
}

/// The last path component of a cwd — the project name a person recognizes.
pub fn project_name(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

/// Distinct project names across the cards, most cards first, then alphabetical.
pub fn projects(cards: &[Card]) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for c in cards {
        *counts.entry(c.project()).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.into_iter().map(|(p, _)| p).collect()
}
