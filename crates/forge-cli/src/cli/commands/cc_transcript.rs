//! Streaming reader for Claude Code session transcripts (`~/.claude/projects/<cwd>/<id>.jsonl`).
//!
//! Transcripts reach hundreds of MB, so nothing here holds the file in memory. Pass one parses each
//! line into a slim node (uuid, parent links, byte offset) and never keeps message bodies; pass two
//! seeks back to just the lines on the active chain and turns them into messages one at a time.
//!
//! The ACTIVE chain is what Claude Code itself would resume: walk `parentUuid` from the last leaf,
//! crossing a compaction boundary through `logicalParentUuid`. The model context after the LAST
//! boundary is the compact summary, then the boundary's preserved segment (the recent messages
//! Claude Code keeps verbatim), then everything recorded after the summary.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};
use forge_types::{Role, ToolCall};
use serde_json::Value;

use self::slim::Slim;

/// How many chain entries past a call to look for its result when deciding a merged message is complete.
const RESULT_LOOKAHEAD: usize = 64;
const MISSING_RESULT: &str = "[no result was recorded for this tool call]";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    User,
    Assistant,
    Boundary,
    Other,
}

struct Node {
    uuid: String,
    parent: Option<String>,
    logical_parent: Option<String>,
    kind: Kind,
    offset: u64,
    len: usize,
    msg_id: Option<String>,
    model: Option<String>,
    is_compact_summary: bool,
    preserved: Option<(String, String)>,
    /// `tool_use` ids an assistant line requests.
    uses: Vec<String>,
    /// `tool_use_id`s a user line answers.
    results: Vec<String>,
}

/// One message ready for the Forge store.
#[derive(Debug, Clone)]
pub(crate) struct Imported {
    pub(crate) role: Role,
    pub(crate) content: String,
    pub(crate) model: Option<String>,
    pub(crate) tool_calls: Vec<ToolCall>,
    pub(crate) tool_call_id: Option<String>,
    /// Part of what Claude Code would send on its next turn.
    pub(crate) live: bool,
    pub(crate) timestamp: Option<i64>,
}

/// A transcript indexed for import. Created by [`Transcript::scan`].
pub(crate) struct Transcript {
    file: File,
    nodes: Vec<Node>,
    chain: Vec<usize>,
    boundary_pos: Option<usize>,
    summary_pos: Option<usize>,
    preserved: Option<(usize, usize)>,
    pub(crate) cwd: Option<String>,
    /// Claude Code's own name for the session (`/rename`, else its auto title).
    pub(crate) title: Option<String>,
    /// Model of the last assistant message on the active chain.
    pub(crate) model: Option<String>,
    pub(crate) lines: usize,
    pub(crate) skipped_lines: usize,
    /// Offset just past the last complete line read: where the next incremental read starts.
    pub(crate) bytes_end: u64,
    /// The last chain entry of the file (the leaf Claude Code would continue from).
    pub(crate) last_uuid: Option<String>,
    /// What the first chain entry hangs off: `None` for a file read from its start, otherwise the
    /// entry before the range, which an incremental read must find equal to its stored leaf.
    pub(crate) root_parent: Option<String>,
}

/// What [`Transcript::for_each_message`] did, for the report the command prints.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImportStats {
    pub(crate) messages: usize,
    pub(crate) live: usize,
    pub(crate) synthetic_results: usize,
    pub(crate) dropped_orphan_results: usize,
}

enum Slot {
    Pending(String),
    Ready(Imported),
}

/// A merged assistant message can span lines where the person typed again while the turn was
/// still running. Its calls' results must still directly follow it, so they are pulled forward
/// past any such text.
fn answer_first(
    queue: &mut VecDeque<Slot>,
    calls: &HashSet<String>,
    out: &mut Normalizer,
    emit: &mut impl FnMut(Imported) -> Result<()>,
) -> Result<()> {
    if calls.is_empty() {
        return Ok(());
    }
    let mut kept = VecDeque::with_capacity(queue.len());
    for slot in queue.drain(..) {
        match slot {
            Slot::Ready(m)
                if m.role == Role::Tool
                    && m.tool_call_id.as_ref().is_some_and(|id| calls.contains(id)) =>
            {
                out.push(m, emit)?;
            }
            other => kept.push_back(other),
        }
    }
    *queue = kept;
    Ok(())
}

impl Transcript {
    /// Pass one: index every non-sidechain line and resolve the active chain.
    pub(crate) fn scan(path: &Path) -> Result<Self> {
        Self::scan_from(path, 0)
    }

    /// [`Self::scan`] over the lines at and after byte `start` (a line boundary): the cheap read
    /// of just what a linked session gained since it was last mirrored.
    pub(crate) fn scan_from(path: &Path, start: u64) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut reader = BufReader::with_capacity(1 << 20, file.try_clone()?);
        reader.seek(SeekFrom::Start(start))?;
        let mut nodes: Vec<Node> = Vec::new();
        let mut by_uuid: HashMap<String, usize> = HashMap::new();
        let (mut cwd, mut custom_title, mut ai_title) = (None, None, None);
        let (mut lines, mut skipped) = (0usize, 0usize);
        let mut offset = start;
        let mut bytes_end = start;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            let at = offset;
            offset += n as u64;
            let complete = buf.ends_with(b"\n");
            let text = buf.trim_ascii();
            if text.is_empty() {
                bytes_end = offset;
                continue;
            }
            let parsed = serde_json::from_slice::<Slim>(text);
            // A last line without its newline that does not parse yet is a write in progress:
            // leave it for the next read instead of counting it as damage.
            if parsed.is_err() && !complete {
                break;
            }
            bytes_end = offset;
            lines += 1;
            let Ok(slim) = parsed else {
                skipped += 1;
                continue;
            };

            if slim.sidechain {
                continue;
            }
            if let Some(t) = slim.custom_title.filter(|t| !t.trim().is_empty()) {
                custom_title = Some(t);
            }
            if let Some(t) = slim.ai_title.filter(|t| !t.trim().is_empty()) {
                ai_title = Some(t);
            }
            let Some(uuid) = slim.uuid else { continue };
            let kind = match (slim.kind.as_deref(), slim.subtype.as_deref()) {
                (Some("user"), _) => Kind::User,
                (Some("assistant"), _) => Kind::Assistant,
                (Some("system"), Some("compact_boundary")) => Kind::Boundary,
                (Some("system" | "attachment"), _) => Kind::Other,
                _ => continue,
            };
            if cwd.is_none() {
                cwd = slim.cwd.filter(|c| !c.is_empty());
            }
            let preserved = slim
                .compact
                .and_then(|c| c.segment)
                .and_then(|s| Some((s.head?, s.tail?)));
            let mut msg = slim.message.unwrap_or_default();
            let ids = msg.content.take().unwrap_or_default();
            by_uuid.insert(uuid.clone(), nodes.len());
            nodes.push(Node {
                uuid,
                parent: slim.parent,
                logical_parent: slim.logical_parent,
                kind,
                offset: at,
                len: n,
                msg_id: msg.id.filter(|i| !i.is_empty()),
                model: msg.model.filter(|m| !m.starts_with('<')),
                is_compact_summary: slim.compact_summary,
                preserved,
                uses: ids.uses,
                results: ids.results,
            });
        }

        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = nodes.len().checked_sub(1);
        while let Some(i) = cur {
            if !seen.insert(i) {
                break;
            }
            chain.push(i);
            let n = &nodes[i];
            cur = n
                .parent
                .as_ref()
                .or(n.logical_parent.as_ref())
                .and_then(|u| by_uuid.get(u).copied());
        }
        chain.reverse();
        let root_parent = chain.first().and_then(|&i| {
            nodes[i]
                .parent
                .clone()
                .or_else(|| nodes[i].logical_parent.clone())
        });
        let last_uuid = nodes.last().map(|n| n.uuid.clone());
        recover_parallel_results(&nodes, &mut chain);

        let boundary_pos = chain.iter().rposition(|&i| nodes[i].kind == Kind::Boundary);
        let summary_pos = boundary_pos.and_then(|b| {
            let rest = &chain[b + 1..];
            rest.iter()
                .position(|&i| nodes[i].kind == Kind::User && nodes[i].is_compact_summary)
                .or_else(|| rest.iter().position(|&i| nodes[i].kind == Kind::User))
                .map(|p| b + 1 + p)
        });
        let preserved = boundary_pos.and_then(|b| {
            let (head, tail) = nodes[chain[b]].preserved.as_ref()?;
            let pos = |u: &str| chain[..b].iter().position(|&i| nodes[i].uuid == u);
            Some((pos(head)?, pos(tail)?)).filter(|(h, t)| h <= t)
        });
        let model = chain.iter().rev().find_map(|&i| {
            nodes[i]
                .model
                .clone()
                .filter(|_| nodes[i].kind == Kind::Assistant)
        });

        Ok(Self {
            file,
            nodes,
            chain,
            boundary_pos,
            summary_pos,
            preserved,
            cwd,
            title: custom_title.or(ai_title),
            model,
            lines,
            skipped_lines: skipped,
            bytes_end,
            last_uuid,
            root_parent,
        })
    }

    /// Roughly how many messages the active chain holds: what a person typed plus Claude's
    /// replies. For the one-line summary `forge switch` prints.
    pub(crate) fn message_count(&self) -> usize {
        let mut seen = HashSet::new();
        self.chain
            .iter()
            .map(|&i| &self.nodes[i])
            .filter(|n| match n.kind {
                Kind::User => !n.is_compact_summary && n.results.is_empty(),
                Kind::Assistant => seen.insert(n.msg_id.clone().unwrap_or_else(|| n.uuid.clone())),
                _ => false,
            })
            .count()
    }

    /// Whether a compaction on the chain keeps a verbatim segment this read could not resolve
    /// (it points before the range read), so its model context cannot be rebuilt from this range.
    pub(crate) fn has_unresolved_preserved_segment(&self) -> bool {
        self.boundary_pos.is_some_and(|b| {
            self.nodes[self.chain[b]].preserved.is_some() && self.preserved.is_none()
        })
    }

    /// Whether the chain holds a compaction boundary.
    pub(crate) fn has_boundary(&self) -> bool {
        self.boundary_pos.is_some()
    }

    /// Whether the active chain carries no conversation at all.
    pub(crate) fn is_empty(&self) -> bool {
        !self
            .chain
            .iter()
            .any(|&i| matches!(self.nodes[i].kind, Kind::User | Kind::Assistant))
    }

    /// Number of compaction boundaries on the active chain.
    pub(crate) fn compactions(&self) -> usize {
        self.chain
            .iter()
            .filter(|&&i| self.nodes[i].kind == Kind::Boundary)
            .count()
    }

    fn live_at(&self, pos: usize) -> bool {
        let Some(b) = self.boundary_pos else {
            return true;
        };
        pos > self.summary_pos.unwrap_or(b)
            || self.preserved.is_some_and(|(h, t)| (h..=t).contains(&pos))
    }

    fn read_node(&self, reader: &mut BufReader<File>, at: &mut u64, node: &Node) -> Result<Value> {
        if node.offset >= *at {
            reader.seek_relative((node.offset - *at) as i64)?;
        } else {
            reader.seek(SeekFrom::Start(node.offset))?;
        }
        let mut buf = vec![0u8; node.len];
        reader.read_exact(&mut buf)?;
        *at = node.offset + node.len as u64;
        serde_json::from_slice(buf.trim_ascii()).context("re-reading a transcript line")
    }

    fn open_reader(&self) -> Result<BufReader<File>> {
        // A cloned handle shares its cursor with every other clone, so rewind it explicitly.
        let mut reader = BufReader::with_capacity(1 << 20, self.file.try_clone()?);
        reader.seek(SeekFrom::Start(0))?;
        Ok(reader)
    }

    /// The compact summary the last boundary produced, if any.
    pub(crate) fn summary(&self) -> Result<Option<String>> {
        let Some(pos) = self.summary_pos else {
            return Ok(None);
        };
        let mut reader = self.open_reader()?;
        let mut at = 0;
        let v = self.read_node(&mut reader, &mut at, &self.nodes[self.chain[pos]])?;
        let (texts, _) = user_blocks(&v["message"]["content"]);
        Ok(Some(texts.join("\n\n")).filter(|s| !s.trim().is_empty()))
    }

    /// The first thing the person typed, for sessions with no name of their own.
    pub(crate) fn first_prompt(&self) -> Result<Option<String>> {
        let mut reader = self.open_reader()?;
        let mut at = 0;
        for (pos, &i) in self.chain.iter().enumerate() {
            let node = &self.nodes[i];
            if node.kind != Kind::User || Some(pos) == self.summary_pos {
                continue;
            }
            let v = self.read_node(&mut reader, &mut at, node)?;
            if v["isMeta"].as_bool() == Some(true) {
                continue;
            }
            let (texts, results) = user_blocks(&v["message"]["content"]);
            let text = texts.join(" ");
            if results.is_empty() && !text.trim().is_empty() && !text.trim_start().starts_with('<')
            {
                return Ok(Some(text));
            }
        }
        Ok(None)
    }

    /// Pass two: stream the active chain as Forge messages, oldest first. Assistant fragments that
    /// share a `message.id` are merged into one message (Claude Code writes one line per content
    /// block, sometimes with a tool result in between); tool results follow the merged message.
    pub(crate) fn for_each_message(
        &self,
        mut emit: impl FnMut(Imported) -> Result<()>,
    ) -> Result<ImportStats> {
        // A merged assistant message is complete once its last fragment AND the results of all its
        // calls have been seen; until then later lines queue behind it. The look-ahead is capped so
        // a result that never arrives cannot hold the rest of the chain back.
        let mut result_pos: HashMap<&str, usize> = HashMap::new();
        for (pos, &i) in self.chain.iter().enumerate() {
            for id in &self.nodes[i].results {
                result_pos.insert(id, pos);
            }
        }
        let mut ready_at: HashMap<&str, usize> = HashMap::new();
        for (pos, &i) in self.chain.iter().enumerate() {
            let node = &self.nodes[i];
            let (Kind::Assistant, Some(id)) = (node.kind, &node.msg_id) else {
                continue;
            };
            let mut at = pos;
            for r in node.uses.iter().filter_map(|u| result_pos.get(u.as_str())) {
                if *r > pos && *r - pos <= RESULT_LOOKAHEAD {
                    at = at.max(*r);
                }
            }
            let slot = ready_at.entry(id).or_insert(at);
            *slot = (*slot).max(at);
        }
        let mut reader = self.open_reader()?;
        let mut at = 0;
        let mut queue: VecDeque<Slot> = VecDeque::new();
        let mut pending: HashMap<String, (Imported, usize)> = HashMap::new();
        let mut out = Normalizer::default();

        for (pos, &i) in self.chain.iter().enumerate() {
            let node = &self.nodes[i];
            let live = self.live_at(pos);
            match node.kind {
                Kind::Boundary | Kind::Other => continue,
                Kind::Assistant => {
                    let v = self.read_node(&mut reader, &mut at, node)?;
                    let key = node.msg_id.clone().unwrap_or_else(|| node.uuid.clone());
                    if !pending.contains_key(&key) {
                        queue.push_back(Slot::Pending(key.clone()));
                        pending.insert(
                            key.clone(),
                            (
                                Imported {
                                    role: Role::Assistant,
                                    content: String::new(),
                                    model: node.model.clone(),
                                    tool_calls: Vec::new(),
                                    tool_call_id: None,
                                    live,
                                    timestamp: timestamp(&v),
                                },
                                0,
                            ),
                        );
                    }
                    let entry = pending.get_mut(&key).expect("inserted above");
                    merge_assistant(&mut entry.0, &v["message"]["content"]);
                    entry.1 = ready_at.get(key.as_str()).copied().unwrap_or(pos);
                }
                Kind::User => {
                    let v = self.read_node(&mut reader, &mut at, node)?;
                    let ts = timestamp(&v);
                    let (texts, results) = user_blocks(&v["message"]["content"]);
                    let is_summary = Some(pos) == self.summary_pos;
                    for (id, content) in results {
                        queue.push_back(Slot::Ready(Imported {
                            role: Role::Tool,
                            content,
                            model: None,
                            tool_calls: Vec::new(),
                            tool_call_id: Some(id),
                            live,
                            timestamp: ts,
                        }));
                    }
                    let text = texts.join("\n\n");
                    if !text.trim().is_empty() {
                        queue.push_back(Slot::Ready(Imported {
                            role: Role::User,
                            content: text,
                            model: None,
                            tool_calls: Vec::new(),
                            tool_call_id: None,
                            live: live && !is_summary,
                            timestamp: ts,
                        }));
                    }
                }
            }
            loop {
                match queue.front() {
                    Some(Slot::Ready(_)) => {
                        if let Some(Slot::Ready(m)) = queue.pop_front() {
                            out.push(m, &mut emit)?;
                        }
                    }
                    Some(Slot::Pending(k)) if pending.get(k).is_some_and(|p| p.1 <= pos) => {
                        if let Some(Slot::Pending(k)) = queue.pop_front() {
                            if let Some((m, _)) = pending.remove(&k) {
                                let calls: HashSet<String> =
                                    m.tool_calls.iter().map(|c| c.id.clone()).collect();
                                out.push(m, &mut emit)?;
                                answer_first(&mut queue, &calls, &mut out, &mut emit)?;
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
        while let Some(slot) = queue.pop_front() {
            match slot {
                Slot::Ready(m) => out.push(m, &mut emit)?,
                Slot::Pending(k) => {
                    if let Some((m, _)) = pending.remove(&k) {
                        out.push(m, &mut emit)?;
                    }
                }
            }
        }
        out.finish(&mut emit)?;
        Ok(out.stats)
    }
}

/// Claude Code records each parallel tool call's result as its own branch, so the walk from the
/// last leaf reaches only one of them. Claude Code's loader pulls the others back in; so do we:
/// a result line off the chain whose call is on it is placed right after that call's message.
fn recover_parallel_results(nodes: &[Node], chain: &mut Vec<usize>) {
    let on_chain: HashSet<usize> = chain.iter().copied().collect();
    let answered: HashSet<&str> = chain
        .iter()
        .flat_map(|&i| nodes[i].results.iter().map(String::as_str))
        .collect();
    let mut off_chain: HashMap<&str, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if n.kind == Kind::User && !on_chain.contains(&i) {
            for id in &n.results {
                off_chain.entry(id).or_insert(i);
            }
        }
    }
    if off_chain.is_empty() {
        return;
    }
    let mut inserts: Vec<(usize, usize)> = Vec::new();
    let mut placed: HashSet<usize> = HashSet::new();
    for (pos, &i) in chain.iter().enumerate() {
        let n = &nodes[i];
        if n.kind != Kind::Assistant {
            continue;
        }
        for id in n.uses.iter().filter(|id| !answered.contains(id.as_str())) {
            let Some(&r) = off_chain.get(id.as_str()) else {
                continue;
            };
            if !placed.insert(r) {
                continue;
            }
            let mut last = pos;
            for (j, &k) in chain.iter().enumerate().skip(pos + 1) {
                match nodes[k].kind {
                    Kind::Assistant if n.msg_id.is_some() && nodes[k].msg_id == n.msg_id => {
                        last = j;
                    }
                    Kind::Assistant => break,
                    Kind::User if nodes[k].results.is_empty() => break,
                    _ => {}
                }
            }
            inserts.push((last + 1, r));
        }
    }
    inserts.sort_by_key(|&(at, _)| std::cmp::Reverse(at));
    for (at, r) in inserts {
        chain.insert(at.min(chain.len()), r);
    }
}

fn timestamp(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v["timestamp"].as_str()?)
        .ok()
        .map(|t| t.timestamp())
}

fn merge_assistant(into: &mut Imported, content: &Value) {
    let mut push_text = |t: &str| {
        if t.trim().is_empty() {
            return;
        }
        if !into.content.is_empty() {
            into.content.push_str("\n\n");
        }
        into.content.push_str(t);
    };
    match content {
        Value::String(s) => push_text(s),
        Value::Array(blocks) => {
            for b in blocks {
                match b["type"].as_str() {
                    Some("text") => push_text(b["text"].as_str().unwrap_or_default()),
                    Some("tool_use") => {
                        let (Some(id), Some(name)) = (b["id"].as_str(), b["name"].as_str()) else {
                            continue;
                        };
                        if !into.tool_calls.iter().any(|c| c.id == id) {
                            into.tool_calls.push(ToolCall {
                                id: id.to_string(),
                                name: name.to_string(),
                                args: b["input"].clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// A user line's visible text blocks and its tool results as `(tool_use_id, text)`.
fn user_blocks(content: &Value) -> (Vec<String>, Vec<(String, String)>) {
    let mut texts = Vec::new();
    let mut results = Vec::new();
    match content {
        Value::String(s) => texts.push(s.clone()),
        Value::Array(blocks) => {
            for b in blocks {
                match b["type"].as_str() {
                    Some("text") => texts.push(b["text"].as_str().unwrap_or_default().to_string()),
                    Some("image") => texts.push("[image]".into()),
                    Some("document") => texts.push("[document]".into()),
                    Some("tool_result") => {
                        if let Some(id) = b["tool_use_id"].as_str() {
                            results.push((id.to_string(), result_text(&b["content"])));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    (texts, results)
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(str::to_string),
                Some("image") => Some("[image]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Final stage: drops empty assistant turns, drops tool results that answer no call, and gives
/// every call that never got a result a placeholder one, so the stored history is a valid
/// call/result round-trip.
#[derive(Default)]
struct Normalizer {
    open: Vec<(String, bool)>,
    stats: ImportStats,
}

impl Normalizer {
    fn push(&mut self, m: Imported, emit: &mut impl FnMut(Imported) -> Result<()>) -> Result<()> {
        match m.role {
            Role::Tool => {
                let id = m.tool_call_id.as_deref().unwrap_or_default();
                let Some(i) = self.open.iter().position(|(o, _)| o == id) else {
                    self.stats.dropped_orphan_results += 1;
                    return Ok(());
                };
                self.open.remove(i);
            }
            Role::Assistant if m.content.is_empty() && m.tool_calls.is_empty() => return Ok(()),
            _ => self.close_open(emit)?,
        }
        if m.role == Role::Assistant {
            self.open = m
                .tool_calls
                .iter()
                .map(|c| (c.id.clone(), m.live))
                .collect();
        }
        self.stats.messages += 1;
        self.stats.live += usize::from(m.live);
        emit(m)
    }

    fn close_open(&mut self, emit: &mut impl FnMut(Imported) -> Result<()>) -> Result<()> {
        for (id, live) in std::mem::take(&mut self.open) {
            self.stats.synthetic_results += 1;
            self.stats.messages += 1;
            self.stats.live += usize::from(live);
            emit(Imported {
                role: Role::Tool,
                content: MISSING_RESULT.into(),
                model: None,
                tool_calls: Vec::new(),
                tool_call_id: Some(id),
                live,
                timestamp: None,
            })?;
        }
        Ok(())
    }

    fn finish(&mut self, emit: &mut impl FnMut(Imported) -> Result<()>) -> Result<()> {
        self.close_open(emit)
    }
}

mod slim;
#[cfg(test)]
mod tests;
