//! Plan and dispatch: one prompt, split into parallel sessions (docs/features/project-board.md §
//! "Plan and dispatch").
//!
//! A **coordinator** session reads the project, splits the user's request into independent work
//! items, and proposes them with the `dispatch_sessions` virtual tool. The user approves the split
//! (all of it, a subset, or asks for a revision) out of band — on the board, `forge dispatch
//! approve`, or any client of the daemon's dispatch routes — and the daemon starts one ordinary
//! session per item, in its own worktree by default. The daemon then tells the coordinator, with
//! a `[dispatch]` follow-up message, each time a session finishes, starts items whose dependencies
//! have succeeded, and asks the coordinator for a final summary when everything is done.
//!
//! Why approval is not a question asked from inside the tool call: a coordinator running on a CLI
//! bridge executes its tools in a separate `forge mcp-serve` process that has no presenter, so a
//! blocking question could never reach the user there. Recording the proposal and ending the turn
//! works identically for direct-API and bridge coordinators, survives the board being closed, and
//! survives a daemon restart (the proposal is in the store).
//!
//! This module is the surface-independent half (ADR-0004): plan parsing and validation, the
//! scheduling and selection rules, every text the models read, and the [`SessionDispatch`] seam the
//! daemon implements. The daemon owns persistence, session creation and delivery.

use std::collections::BTreeSet;

use forge_types::ToolCall;

pub use forge_store::{dispatch_item_status as item_status, dispatch_status};

use crate::{CoreError, Session};

/// The virtual tool a coordinator calls to propose its split.
pub const DISPATCH_SESSIONS_TOOL: &str = "dispatch_sessions";
/// Items one dispatch may propose unless the host says otherwise.
pub const DEFAULT_MAX_ITEMS: usize = 8;
/// Absolute ceiling, whatever the host asks for — each item is a whole agent session.
pub const MAX_ITEMS_HARD: usize = 12;
/// Sessions of one dispatch that may run at once unless the request says otherwise.
pub const DEFAULT_MAX_RUNNING: usize = 4;
/// Upper bound for a requested `max_running`.
pub const MAX_RUNNING_HARD: usize = 8;
/// Longest accepted item title, in characters (it becomes the session title).
pub const MAX_TITLE_CHARS: usize = 80;
/// Longest accepted item prompt, in bytes — the same order as a fleet message.
pub const MAX_PROMPT_BYTES: usize = 16 * 1024;
/// How much of a finished session's final reply is forwarded to the coordinator.
pub const REPORT_MAX_CHARS: usize = 2000;
/// Sender label on every `[dispatch]` message the daemon queues for a coordinator.
pub const SENDER_LABEL: &str = "dispatch";

/// One proposed work item, as the coordinator wrote it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DispatchItemSpec {
    pub title: String,
    pub prompt: String,
    /// 1-based numbers of the items that must succeed before this one starts.
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

/// A validated proposal.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DispatchPlan {
    pub summary: String,
    pub items: Vec<DispatchItemSpec>,
}

/// What the host recorded for a proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalReceipt {
    pub dispatch_id: String,
    pub items: usize,
}

/// Host capability wired into coordinator sessions only (the daemon driver in-process, or the CLI
/// bridge over HTTP). A session without one never sees `dispatch_sessions`, so an ordinary session
/// or a dispatched worker cannot start a dispatch of its own.
#[async_trait::async_trait]
pub trait SessionDispatch: Send + Sync {
    /// Items this coordinator may propose (clamped to [`MAX_ITEMS_HARD`] by [`parse_plan`]).
    fn max_items(&self) -> usize;
    /// Record `plan` as this coordinator's proposal, replacing one the user has not approved yet.
    /// `Err` is shown to the model as the tool result.
    async fn propose(&self, plan: DispatchPlan) -> Result<ProposalReceipt, String>;
}

/// The `ToolSpec` advertised to a coordinator.
pub fn dispatch_sessions_spec(max_items: usize) -> forge_provider::ToolSpec {
    let max_items = max_items.clamp(1, MAX_ITEMS_HARD);
    forge_provider::ToolSpec {
        name: DISPATCH_SESSIONS_TOOL.to_string(),
        description: format!(
            "Propose how to split the user's request into work items that separate Forge \
             sessions do in parallel. Each item becomes its own session. The user reviews the \
             split before anything starts; after calling this, end your turn and wait for a \
             [dispatch] message with their decision. Calling it again before approval replaces \
             the proposal. At most {max_items} items; one item is a valid answer when the work \
             does not split cleanly."
        ),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "one paragraph: the overall goal and how the items divide it"
                },
                "items": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": max_items,
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": {
                                "type": "string",
                                "description": "short name for this part (becomes the session title)"
                            },
                            "prompt": {
                                "type": "string",
                                "description": "self-contained instructions for a fresh agent: goal, files or areas, constraints, how to verify, what done means"
                            },
                            "depends_on": {
                                "type": "array",
                                "items": { "type": "integer", "minimum": 1 },
                                "description": "1-based numbers of items that must succeed before this one starts; omit when independent"
                            }
                        },
                        "required": ["title", "prompt"]
                    }
                }
            },
            "required": ["summary", "items"]
        }),
    }
}

/// Parse and validate `dispatch_sessions` arguments. Every rejection names the fix, because the
/// model reads it and calls again.
pub fn parse_plan(args: &serde_json::Value, max_items: usize) -> Result<DispatchPlan, String> {
    let max_items = max_items.clamp(1, MAX_ITEMS_HARD);
    let summary = args
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if summary.is_empty() {
        return Err("`summary` is required: one paragraph describing the overall goal".into());
    }
    let Some(raw_items) = args.get("items").and_then(|v| v.as_array()) else {
        return Err("`items` must be an array of {title, prompt, depends_on?}".into());
    };
    if raw_items.is_empty() {
        return Err("`items` is empty: propose at least one item".into());
    }
    if raw_items.len() > max_items {
        return Err(format!(
            "{} items proposed but at most {max_items} are allowed: merge related items",
            raw_items.len()
        ));
    }
    let n = raw_items.len();
    let mut items = Vec::with_capacity(n);
    let mut titles = BTreeSet::new();
    for (i, raw) in raw_items.iter().enumerate() {
        let number = i + 1;
        let title = raw
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .replace(['\n', '\r'], " ");
        if title.is_empty() {
            return Err(format!("item {number} has no title"));
        }
        if title.chars().count() > MAX_TITLE_CHARS {
            return Err(format!(
                "item {number}'s title is longer than {MAX_TITLE_CHARS} characters: shorten it"
            ));
        }
        if !titles.insert(title.to_lowercase()) {
            return Err(format!(
                "item {number} repeats the title \"{title}\": give every item a distinct title"
            ));
        }
        let prompt = raw
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if prompt.is_empty() {
            return Err(format!("item {number} (\"{title}\") has no prompt"));
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(format!(
                "item {number}'s prompt is {} bytes, over the {MAX_PROMPT_BYTES}-byte limit",
                prompt.len()
            ));
        }
        let mut depends_on = BTreeSet::new();
        if let Some(deps) = raw.get("depends_on") {
            let Some(deps) = deps.as_array() else {
                return Err(format!(
                    "item {number}'s depends_on must be an array of item numbers"
                ));
            };
            for d in deps {
                let Some(d) = d.as_u64().map(|d| d as usize) else {
                    return Err(format!(
                        "item {number}'s depends_on must contain item numbers (1..={n})"
                    ));
                };
                if d == 0 || d > n {
                    return Err(format!(
                        "item {number} depends on item {d}, which does not exist (1..={n})"
                    ));
                }
                if d == number {
                    return Err(format!("item {number} depends on itself"));
                }
                depends_on.insert(d);
            }
        }
        items.push(DispatchItemSpec {
            title,
            prompt,
            depends_on: depends_on.into_iter().collect(),
        });
    }
    if let Some(cycle) = find_cycle(&items) {
        return Err(format!(
            "the dependencies form a cycle through item {cycle}: an item cannot wait on itself"
        ));
    }
    Ok(DispatchPlan { summary, items })
}

/// A 1-based item on a dependency cycle, if any.
fn find_cycle(items: &[DispatchItemSpec]) -> Option<usize> {
    // 0 = unvisited, 1 = on the stack, 2 = done.
    fn visit(i: usize, items: &[DispatchItemSpec], mark: &mut [u8]) -> Option<usize> {
        match mark[i] {
            1 => return Some(i + 1),
            2 => return None,
            _ => {}
        }
        mark[i] = 1;
        for d in &items[i].depends_on {
            if let Some(c) = visit(d - 1, items, mark) {
                return Some(c);
            }
        }
        mark[i] = 2;
        None
    }
    let mut mark = vec![0u8; items.len()];
    (0..items.len()).find_map(|i| visit(i, items, &mut mark))
}

/// The user's approval, resolved against the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Approval {
    /// Items that will run (immediately or once their dependencies succeed), ascending.
    pub queued: Vec<usize>,
    /// Items the user did not select.
    pub skipped: Vec<usize>,
    /// Selected items that cannot run because something they depend on was not selected.
    pub dropped_for_deps: Vec<usize>,
}

/// Resolve a selection (`None` = every item) against `deps[i]` = the 1-based dependencies of item
/// `i + 1`. An item whose dependency is not going to run cannot run either, transitively.
pub fn resolve_selection(
    deps: &[Vec<usize>],
    selected: Option<&[usize]>,
) -> Result<Approval, String> {
    let n = deps.len();
    let chosen: BTreeSet<usize> = match selected {
        None => (1..=n).collect(),
        Some(sel) => {
            for s in sel {
                if *s == 0 || *s > n {
                    return Err(format!("item {s} does not exist (1..={n})"));
                }
            }
            sel.iter().copied().collect()
        }
    };
    if chosen.is_empty() {
        return Err("select at least one item, or cancel the dispatch".into());
    }
    let mut runnable = chosen.clone();
    loop {
        let before = runnable.clone();
        runnable.retain(|i| deps[i - 1].iter().all(|d| before.contains(d)));
        if runnable.len() == before.len() {
            break;
        }
    }
    Ok(Approval {
        queued: runnable.iter().copied().collect(),
        skipped: (1..=n).filter(|i| !chosen.contains(i)).collect(),
        dropped_for_deps: chosen.difference(&runnable).copied().collect(),
    })
}

/// One item's live state, for [`schedule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemState {
    pub index: usize,
    pub status: String,
    pub depends_on: Vec<usize>,
}

/// What the daemon should do next for a running dispatch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Schedule {
    /// Queued items to start now, ascending, within the running cap.
    pub start: Vec<usize>,
    /// Queued items that can never start because a dependency did not succeed.
    pub cancel: Vec<usize>,
}

/// Decide which queued items start and which are cancelled. Dependencies count as satisfied once
/// they succeeded or were merged; any other terminal state blocks the dependent, transitively.
pub fn schedule(items: &[ItemState], max_running: usize) -> Schedule {
    let mut status: Vec<String> = items.iter().map(|i| i.status.clone()).collect();
    let pos = |index: usize| items.iter().position(|i| i.index == index);
    let mut out = Schedule::default();
    loop {
        let mut changed = false;
        for (k, item) in items.iter().enumerate() {
            if status[k] != item_status::QUEUED {
                continue;
            }
            let blocked = item.depends_on.iter().any(|d| {
                pos(*d).is_some_and(|p| {
                    item_status::is_terminal(&status[p]) && !satisfies_dependency(&status[p])
                })
            });
            if blocked {
                status[k] = item_status::CANCELLED.to_string();
                out.cancel.push(item.index);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut running = status
        .iter()
        .filter(|s| s.as_str() == item_status::RUNNING)
        .count();
    for (k, item) in items.iter().enumerate() {
        if running >= max_running.max(1) {
            break;
        }
        if status[k] != item_status::QUEUED {
            continue;
        }
        let ready = item
            .depends_on
            .iter()
            .all(|d| pos(*d).is_some_and(|p| satisfies_dependency(&status[p])));
        if ready {
            status[k] = item_status::RUNNING.to_string();
            out.start.push(item.index);
            running += 1;
        }
    }
    out.start.sort_unstable();
    out.cancel.sort_unstable();
    out
}

fn satisfies_dependency(status: &str) -> bool {
    status == item_status::SUCCEEDED || status == item_status::MERGED
}

/// Whether every item reached a terminal state.
pub fn all_finished<'a>(statuses: impl IntoIterator<Item = &'a str>) -> bool {
    statuses.into_iter().all(item_status::is_terminal)
}

/// The first prompt a coordinator session receives. It names none of the words the offline mock
/// provider keys on, so a mock coordinator reacts only to the user's own request text.
pub fn coordinator_prompt(
    user_prompt: &str,
    cwd: &str,
    max_items: usize,
    worktree: bool,
) -> String {
    let max_items = max_items.clamp(1, MAX_ITEMS_HARD);
    let isolation = if worktree {
        "in its own git worktree and branch, so parallel sessions never share a working tree"
    } else {
        "in the same working directory as the others, so items must not edit the same files"
    };
    format!(
        "You are the coordinator of a Forge dispatch for the project at {cwd}. The user asked:\n\n\
         <request>\n{request}\n</request>\n\n\
         Split this into work that separate Forge sessions do in parallel, then coordinate them. \
         You do not implement anything yourself and you do not edit files.\n\n\
         1. Read enough of the project to split the work well: its layout, the code the request \
         touches, and how it is built and verified.\n\
         2. Decide the split. Each item becomes its own session {isolation}.\n\
         - Make items independent. Parallel sessions that change the same files conflict when \
         their work is merged, so give each item its own files or area.\n\
         - Prefer a few substantial items over many small ones, at most {max_items}. If the work \
         does not split cleanly, propose one item; that is a correct answer.\n\
         - Use depends_on only when an item needs another item's result first. It starts after \
         those items succeed.\n\
         3. Write each item's prompt for a fresh agent with no other context: the goal, the files \
         or areas involved, constraints, the exact command that verifies the work, and what done \
         means. Open an item that changes the project with the instruction itself, starting with a \
         verb such as Add, Implement, Fix, Refactor, Update, Remove, Rename, Create, Write or \
         Change. End an item that only investigates, verifies or reports with the sentence \
         \"Do not edit files.\"\n\
         4. Call dispatch_sessions once with a one-paragraph summary and the items, then end your \
         turn. The user reviews the split before anything starts.\n\n\
         Afterwards you receive messages that start with \"[dispatch]\": the user's decision, and a \
         report each time a session finishes. If the user asks for changes, call \
         dispatch_sessions again with the revised items. If a finished session's report affects \
         one still running, tell that session with message_session. When every session has \
         finished, summarize for the user what each one did, whether it succeeded, any overlap or \
         conflict between them, and the order to merge them in.",
        request = user_prompt.trim(),
    )
}

/// What a dispatched worker needs to know about its place in the dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerContext<'a> {
    pub summary: &'a str,
    /// The coordinator's display title.
    pub coordinator_title: &'a str,
    pub coordinator_id: &'a str,
    /// 1-based.
    pub index: usize,
    pub total: usize,
    pub title: &'a str,
    pub prompt: &'a str,
    /// `(number, title)` of every other item.
    pub siblings: &'a [(usize, &'a str)],
    pub worktree: bool,
}

/// The first prompt a dispatched worker session receives: its own item, then its place in the whole.
pub fn worker_prompt(ctx: &WorkerContext<'_>) -> String {
    let others = if ctx.siblings.is_empty() {
        "none — this is the only part".to_string()
    } else {
        ctx.siblings
            .iter()
            .map(|(n, t)| format!("{n}. {t}"))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let isolation = if ctx.worktree {
        "You work in your own git worktree. Your changes are merged back after review, so stay on \
         the branch you were given."
    } else {
        "Other sessions edit this same working directory at the same time, so change only the \
         files your part needs."
    };
    format!(
        "{prompt}\n\n---\n\
         You are session {index} of {total} in a Forge dispatch coordinated by \"{coordinator}\" \
         ({coord_id}). The overall goal: {summary}\n\
         Your part: {title}.\n\
         The other parts, handled by other sessions: {others}.\n\
         {isolation}\n\
         Stay within your part. If you are blocked on another part, or you change something \
         another part relies on, tell the coordinator with message_session (target \"{coord_id}\"). \
         When your part is done and verified, end with a short report: what you changed, how you \
         verified it, and anything the coordinator should know.",
        prompt = ctx.prompt.trim(),
        index = ctx.index,
        total = ctx.total,
        coordinator = ctx.coordinator_title,
        coord_id = short_id(ctx.coordinator_id),
        summary = ctx.summary.trim(),
        title = ctx.title,
    )
}

/// The tool result a coordinator reads after its proposal was recorded.
pub fn proposal_recorded_result(receipt: &ProposalReceipt) -> String {
    let n = receipt.items;
    format!(
        "Proposal recorded: {n} session{s}. The user is reviewing the split now. End your turn \
         with one line saying the split is ready for review. Nothing starts until they approve; \
         their decision arrives as a [dispatch] message.",
        s = plural(n)
    )
}

/// `[dispatch]` message: the user approved.
pub fn approved_message(
    started: &[(usize, &str, &str)],
    waiting: &[(usize, &str, &[usize])],
    not_started: &[(usize, &str)],
) -> String {
    let mut out = String::from("[dispatch] The user approved the split.");
    if !started.is_empty() {
        out.push_str("\nStarted now:");
        for (n, title, id) in started {
            out.push_str(&format!("\n- {n}. {title} ({})", short_id(id)));
        }
    }
    if !waiting.is_empty() {
        out.push_str("\nWaiting to start:");
        for (n, title, deps) in waiting {
            if deps.is_empty() {
                out.push_str(&format!("\n- {n}. {title} (for a free slot)"));
            } else {
                out.push_str(&format!("\n- {n}. {title} (after {})", join_numbers(deps)));
            }
        }
    }
    if !not_started.is_empty() {
        out.push_str("\nNot started:");
        for (n, title) in not_started {
            out.push_str(&format!("\n- {n}. {title}"));
        }
    }
    out.push_str(
        "\nYou will get a report as each session finishes. Nothing to do until then: end your \
         turn with one line.",
    );
    out
}

/// `[dispatch]` message: the user wants a different split.
pub fn revise_message(feedback: &str) -> String {
    format!(
        "[dispatch] The user wants the split changed before anything starts:\n\n{}\n\nRevise the \
         items and call dispatch_sessions again.",
        feedback.trim()
    )
}

/// `[dispatch]` message: the user cancelled.
pub fn cancelled_message(still_running: usize) -> String {
    let running = match still_running {
        0 => String::new(),
        1 => " The 1 session already running keeps running.".to_string(),
        n => format!(" The {n} sessions already running keep running."),
    };
    format!(
        "[dispatch] The user cancelled the dispatch. No more sessions will start.{running} End \
         your turn with one line."
    )
}

/// What the daemon knows about a session that just finished a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedReport<'a> {
    pub index: usize,
    pub total: usize,
    pub title: &'a str,
    pub session_id: &'a str,
    /// `"success"` or `"failed"` (the snapshot's `last_turn_outcome`).
    pub outcome: &'a str,
    /// The snapshot's `last_stop_reason`.
    pub stop_reason: Option<&'a str>,
    /// The session's final reply, untruncated; trimmed to [`REPORT_MAX_CHARS`] here.
    pub last_reply: &'a str,
    pub still_running: usize,
    pub still_waiting: usize,
    /// Items that will not start now because this one did not succeed.
    pub cancelled: &'a [(usize, &'a str)],
}

/// `[dispatch]` message: one session finished.
pub fn item_finished_message(r: &FinishedReport<'_>) -> String {
    let verdict = if r.outcome == "success" {
        "succeeded".to_string()
    } else {
        format!(
            "stopped without finishing ({})",
            r.stop_reason
                .map_or("failed".to_string(), |s| s.replace('_', " "))
        )
    };
    let mut out = format!(
        "[dispatch] Session {}/{} \"{}\" ({}) {verdict}. Still running: {}. Waiting to start: {}.",
        r.index,
        r.total,
        r.title,
        short_id(r.session_id),
        r.still_running,
        r.still_waiting,
    );
    if !r.cancelled.is_empty() {
        let list = r
            .cancelled
            .iter()
            .map(|(n, t)| format!("{n}. {t}"))
            .collect::<Vec<_>>()
            .join("; ");
        out.push_str(&format!(
            "\nNot starting, because a session they depend on did not succeed: {list}."
        ));
    }
    let reply = r.last_reply.trim();
    if reply.is_empty() {
        out.push_str("\nIt ended without a final reply.");
    } else {
        out.push_str(&format!(
            "\nIts final report:\n{}",
            forge_types::truncate_ellipsis(reply, REPORT_MAX_CHARS)
        ));
    }
    out.push_str(
        "\n\nIf this affects a session still running, tell it with message_session. Otherwise \
         end your turn with one line.",
    );
    out
}

/// `[dispatch]` message: every session finished. `rows` = `(number, title, item status)`.
pub fn all_finished_message(rows: &[(usize, &str, &str)]) -> String {
    let mut out = String::from("[dispatch] All sessions have finished:");
    for (n, title, status) in rows {
        out.push_str(&format!("\n- {n}. {title}: {}", status_words(status)));
    }
    out.push_str(
        "\n\nSummarize for the user what each session did, which succeeded, any overlap or \
         conflict between them, and the order to merge them in. Do not edit files.",
    );
    out
}

/// An item status as a person reads it.
pub fn status_words(status: &str) -> &'static str {
    match status {
        item_status::PROPOSED => "proposed",
        item_status::SKIPPED => "not selected",
        item_status::QUEUED => "waiting to start",
        item_status::RUNNING => "running",
        item_status::SUCCEEDED => "succeeded",
        item_status::FAILED => "stopped without finishing",
        item_status::STOPPED => "stopped (its session ended)",
        item_status::CANCELLED => "cancelled",
        item_status::MERGED => "merged",
        item_status::DISCARDED => "discarded",
        _ => "unknown",
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn join_numbers(ns: &[usize]) -> String {
    ns.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl Session {
    /// Make this session a dispatch coordinator (composition root — `forge serve` wires this only
    /// into sessions it started with `POST /api/dispatch`). `None` leaves `dispatch_sessions`
    /// unadvertised.
    pub fn set_session_dispatch(&mut self, dispatch: Option<std::sync::Arc<dyn SessionDispatch>>) {
        self.dispatch = dispatch;
    }

    /// Whether this session coordinates a dispatch.
    pub fn is_dispatch_coordinator(&self) -> bool {
        self.dispatch.is_some()
    }

    /// Handle a `dispatch_sessions` call from a coordinator: validate the plan and hand it to the
    /// host, which records it for the user's approval.
    pub(crate) async fn dispatch_sessions(
        &mut self,
        msg_id: &str,
        call: &ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let (result, ok) = match self.dispatch.clone() {
            None => (
                "error: dispatch_sessions is only available to a dispatch coordinator session"
                    .to_string(),
                false,
            ),
            Some(host) => match parse_plan(&call.args, host.max_items()) {
                Err(e) => (format!("error: {e}"), false),
                Ok(plan) => match host.propose(plan).await {
                    Ok(receipt) => (proposal_recorded_result(&receipt), true),
                    Err(e) => (format!("error: {e}"), false),
                },
            },
        };
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
