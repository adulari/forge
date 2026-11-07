//! Checkpoint compaction at a landed slice (docs/features/context-compaction.md, "Checkpoint at a
//! landed slice").
//!
//! A slice has landed when the agent created a commit in this session and a check (test, lint,
//! typecheck, build) passed after the last edit and before that commit. The next step then starts
//! lean: the slice behind the commit is folded into a summary the harness writes itself, from facts
//! it already holds, so the fold costs no model call. Old reads of files changed since are folded
//! first. Nothing is lost: the rows stay in the store, the summary carries the episode index, and
//! `recall` reads them back.

use std::collections::BTreeSet;
use std::path::Path;

use forge_types::{Message, Role, ToolCall};

use crate::compaction_budget::CompactionBudget;
use crate::compaction_input::fold_inputs;
use crate::compaction_summary::{head_tail, render_verbatim_block_within, SUMMARY_PREFIX};
use crate::completion::{classify_tool, VerificationObservation};
use crate::Session;

pub(crate) const CHECKPOINT_HEADING: &str = "## Checkpoint: slice landed";
const EARLIER_SLICES: &str = "### Landed earlier this session";
const EARLIER_SUMMARY: &str = "### Earlier summary (carried forward)";
/// Closes the summary; a carried model-written body ends where it starts.
const FOOTER: &str = "The reads, edits and tool output of this slice were folded out of your context. They are still stored: `recall` searches or reads them (see the episode index below).";
const MAX_EARLIER_SLICES: usize = 15;
const MAX_CHECKS: usize = 8;
const MAX_FILES: usize = 30;
const MAX_DIFFSTAT_LINES: usize = 25;
const BODY_CHARS: usize = 1_200;
const CARRIED_CHARS: usize = 12_000;
/// A superseded read shorter than this saves nothing worth the marker.
const SUPERSEDED_MIN_CHARS: usize = 400;

/// What the session did since its last checkpoint, read off the tool calls that finished.
#[derive(Debug, Default)]
pub(crate) struct CheckpointTracker {
    edited: Vec<String>,
    /// Passing check commands since the last edit.
    passed: Vec<String>,
    /// Check families whose latest run failed.
    failing: BTreeSet<&'static str>,
    landed: Option<Landed>,
}

/// A commit the agent made with a passing check behind it.
#[derive(Debug, Clone)]
pub(crate) struct Landed {
    call_id: String,
    checks: Vec<String>,
    edited: Vec<String>,
}

impl CheckpointTracker {
    /// Record one finished direct-path call. `ok` is the tool's own verdict (a shell exit 0).
    pub(crate) fn observe(&mut self, call: &ToolCall, ok: bool) {
        let name = call.name.as_str();
        if name.ends_with("shell") || name.ends_with("exec_command") {
            self.observe_shell(call, ok);
            return;
        }
        if ok && classify_tool(name, &call.args.to_string()) == VerificationObservation::Mutation {
            self.passed.clear();
            self.landed = None;
            for path in edit_paths(&call.args) {
                if !self.edited.contains(&path) {
                    self.edited.push(path);
                }
            }
        }
    }

    fn observe_shell(&mut self, call: &ToolCall, ok: bool) {
        let Some(command) = forge_types::shell_command_text(&call.args) else {
            return;
        };
        // A commit message that says "tests" would classify as a check, so commits come first.
        if is_git_commit(&command) {
            if !ok {
                return;
            }
            let verified = !self.passed.is_empty() && self.failing.is_empty();
            self.landed = verified.then(|| Landed {
                call_id: call.id.clone(),
                checks: std::mem::take(&mut self.passed),
                edited: std::mem::take(&mut self.edited),
            });
            self.passed.clear();
            self.edited.clear();
            return;
        }
        match classify_tool(&call.name, &call.args.to_string()) {
            VerificationObservation::Check(family) => {
                if ok {
                    self.failing.remove(family.label());
                    self.pass(command);
                } else {
                    self.failing.insert(family.label());
                }
            }
            VerificationObservation::BatchCheck { family, at } if ok => {
                self.failing.remove(family.label());
                let entry = command.lines().nth(at - 1).unwrap_or(&command).to_string();
                self.pass(entry);
            }
            _ => {}
        }
    }

    fn pass(&mut self, command: String) {
        let command = command.trim().to_string();
        if !self.passed.contains(&command) && self.passed.len() < MAX_CHECKS {
            self.passed.push(command);
        }
    }

    pub(crate) fn take_landed(&mut self) -> Option<Landed> {
        self.landed.take()
    }
}

/// Paths an edit-class tool call writes.
fn edit_paths(args: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut add = |value: &serde_json::Value| {
        if let Some(path) = value.as_str() {
            out.push(normalize_path(path));
        }
    };
    for key in ["path", "file_path", "to"] {
        if let Some(value) = args.get(key) {
            add(value);
        }
    }
    if let Some(paths) = args.get("paths").and_then(|v| v.as_array()) {
        paths.iter().for_each(&mut add);
    }
    if let Some(edits) = args.get("edits").and_then(|v| v.as_array()) {
        edits
            .iter()
            .filter_map(|e| e.get("path"))
            .for_each(&mut add);
    }
    out
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_start_matches("./").to_string()
}

/// Whether a shell command creates a commit: some `git ... commit` segment of the command line.
fn is_git_commit(command: &str) -> bool {
    command
        .split(['\n', ';', '|', '&'])
        .any(|segment| git_subcommand(segment) == Some("commit") && !segment.contains("--dry-run"))
}

fn git_subcommand(segment: &str) -> Option<&str> {
    let mut words = segment.split_whitespace();
    if words.next()? != "git" {
        return None;
    }
    while let Some(word) = words.next() {
        match word {
            "-C" | "-c" | "--git-dir" | "--work-tree" => {
                words.next();
            }
            _ if word.starts_with('-') => {}
            _ => return Some(word),
        }
    }
    None
}

/// Replace the result of every `read_file` of a path that a later call in `messages` edited, and
/// return how many characters that released. The marker says what happened, so the model does not
/// mistake the gap for an error and reads the file again only if it needs the current text.
pub(crate) fn fold_superseded_reads(messages: &mut [Message]) -> usize {
    let mut reads: Vec<(usize, String, String)> = Vec::new();
    let mut edits: Vec<(usize, String)> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        for call in &message.tool_calls {
            if call.name.ends_with("read_file") {
                if let Some(path) = call.args.get("path").and_then(|v| v.as_str()) {
                    reads.push((index, call.id.clone(), normalize_path(path)));
                }
            } else if classify_tool(&call.name, &call.args.to_string())
                == VerificationObservation::Mutation
            {
                edits.extend(edit_paths(&call.args).into_iter().map(|p| (index, p)));
            }
        }
    }
    let mut released = 0usize;
    for (read_at, id, path) in reads {
        if !edits
            .iter()
            .any(|(at, edited)| *at > read_at && *edited == path)
        {
            continue;
        }
        let result = messages[read_at..]
            .iter_mut()
            .find(|m| m.role == Role::Tool && m.tool_call_id.as_deref() == Some(id.as_str()));
        if let Some(result) = result {
            if result.content.len() >= SUPERSEDED_MIN_CHARS {
                let marker = format!(
                    "[superseded: {path} was read here and changed later in this slice; read it \
                     again for its current text]"
                );
                released += result.content.len().saturating_sub(marker.len());
                result.content = marker;
            }
        }
    }
    released
}

/// The git facts of the commit a slice landed with.
#[derive(Debug, Default)]
struct CommitFacts {
    sha: String,
    subject: String,
    body: String,
    diffstat: String,
}

fn git_output(root: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn commit_facts(root: &Path) -> Option<CommitFacts> {
    let sha = git_output(root, &["rev-parse", "--short", "HEAD"])?;
    let subject = git_output(root, &["log", "-1", "--format=%s", "HEAD"])?;
    let body = git_output(root, &["log", "-1", "--format=%b", "HEAD"]).unwrap_or_default();
    let stat = git_output(root, &["show", "--stat", "--format=", "HEAD"]).unwrap_or_default();
    let lines: Vec<&str> = stat.lines().collect();
    let diffstat = if lines.len() > MAX_DIFFSTAT_LINES {
        let mut kept = lines[..MAX_DIFFSTAT_LINES - 1].join("\n");
        kept.push_str(&format!(
            "\n… {} more files\n{}",
            lines.len() - MAX_DIFFSTAT_LINES,
            lines.last().unwrap_or(&"")
        ));
        kept
    } else {
        stat
    };
    Some(CommitFacts {
        sha,
        subject,
        body,
        diffstat,
    })
}

/// One line saying what a file is for, when it says so itself: a module doc, a markdown title or
/// a docstring in its first lines.
pub(crate) fn file_purpose(root: &Path, path: &str) -> Option<String> {
    let text = std::fs::read_to_string(root.join(path)).ok()?;
    for line in text.lines().take(12) {
        let line = line.trim();
        let candidate = line
            .strip_prefix("//!")
            .or_else(|| line.strip_prefix("# "))
            .or_else(|| line.strip_prefix("\"\"\""))
            .or_else(|| line.strip_prefix("/**"))
            .map(str::trim)
            .filter(|c| !c.is_empty());
        if let Some(candidate) = candidate {
            return Some(candidate.chars().take(110).collect());
        }
    }
    None
}

/// The previous summary split into what a checkpoint carries on: earlier slice lines (from an
/// earlier checkpoint) and the model-written body beneath them.
fn carried(previous: Option<&str>) -> (Vec<String>, Option<String>) {
    let Some(previous) = previous.map(|p| p.trim()).filter(|p| !p.is_empty()) else {
        return (Vec::new(), None);
    };
    let previous = previous
        .strip_prefix(SUMMARY_PREFIX)
        .unwrap_or(previous)
        .trim();
    if !previous.starts_with(CHECKPOINT_HEADING) {
        return (Vec::new(), Some(previous.to_string()));
    }
    let mut slices: Vec<String> = Vec::new();
    if let Some(commit) = previous.lines().find(|l| l.starts_with("Commit ")) {
        slices.push(commit.trim_start_matches("Commit ").replacen(": ", " ", 1));
    }
    if let Some(at) = previous.find(EARLIER_SLICES) {
        slices.extend(
            previous[at + EARLIER_SLICES.len()..]
                .lines()
                .take_while(|l| !l.starts_with("###") && !l.starts_with("## "))
                .filter_map(|l| l.strip_prefix("- "))
                .map(str::to_string),
        );
    }
    let body = previous.find(EARLIER_SUMMARY).map(|at| {
        let rest = &previous[at + EARLIER_SUMMARY.len()..];
        let end = rest.find(FOOTER).unwrap_or(rest.len());
        rest[..end].trim().to_string()
    });
    (slices, body.filter(|b| !b.is_empty()))
}

/// Everything the checkpoint summary states.
pub(crate) struct Facts<'a> {
    pub sha: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    pub diffstat: &'a str,
    pub files: Vec<(String, Option<String>)>,
    pub checks: &'a [String],
    pub open_tasks: Vec<String>,
    pub previous: Option<&'a str>,
}

pub(crate) fn render_summary(facts: &Facts<'_>) -> String {
    let mut out = format!(
        "{CHECKPOINT_HEADING}\nCommit {}: {}\n",
        facts.sha, facts.subject
    );
    let body = facts.body.trim();
    if !body.is_empty() {
        out.push_str(&head_tail(body, BODY_CHARS));
        out.push('\n');
    }
    if !facts.diffstat.is_empty() {
        out.push_str(&format!("\n### Diffstat\n{}\n", facts.diffstat));
    }
    if !facts.files.is_empty() {
        out.push_str("\n### Files touched\n");
        for (path, purpose) in facts.files.iter().take(MAX_FILES) {
            match purpose {
                Some(purpose) => out.push_str(&format!("- {path}: {purpose}\n")),
                None => out.push_str(&format!("- {path}\n")),
            }
        }
    }
    out.push_str("\n### Verification that passed (after the last edit, before the commit)\n");
    for check in facts.checks {
        out.push_str(&format!("- `{}`\n", check.replace('\n', " ")));
    }
    out.push_str("\n### Open tasks\n");
    if facts.open_tasks.is_empty() {
        out.push_str("none recorded\n");
    }
    for task in &facts.open_tasks {
        out.push_str(&format!("- {task}\n"));
    }
    let (mut earlier, carried_body) = carried(facts.previous);
    earlier.truncate(MAX_EARLIER_SLICES);
    if !earlier.is_empty() {
        out.push_str(&format!("\n{EARLIER_SLICES}\n"));
        for slice in &earlier {
            out.push_str(&format!("- {slice}\n"));
        }
    }
    if let Some(body) = carried_body {
        out.push_str(&format!(
            "\n{EARLIER_SUMMARY}\n{}\n",
            head_tail(&body, CARRIED_CHARS)
        ));
    }
    out.push('\n');
    out.push_str(FOOTER);
    out
}

fn k(tokens: u64) -> String {
    if tokens < 10_000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        format!("{}k", (tokens + 500) / 1000)
    }
}

impl Session {
    /// At a step boundary: if the last slice landed (a commit after a passing check) and the live
    /// context is above `[compact] checkpoint_min_tokens`, fold the slice. No model call.
    pub(crate) async fn checkpoint_if_landed(&mut self) {
        if !self.config.compact.checkpoint {
            self.checkpoint.take_landed();
            return;
        }
        if self.checkpoint.landed.is_none() {
            return;
        }
        let min = self.config.compact.checkpoint_min_tokens;
        let before_tokens = self.estimated_transcript_tokens();
        if before_tokens < min {
            self.checkpoint.take_landed();
            return;
        }
        let Some(landed) = self.checkpoint.take_landed() else {
            return;
        };
        let Some(round) = self.transcript.iter().position(|m| {
            m.role == Role::Assistant && m.tool_calls.iter().any(|c| c.id == landed.call_id)
        }) else {
            return;
        };
        if round < 2 {
            return;
        }
        let Some(facts) = commit_facts(self.workspace_root()) else {
            return;
        };
        let released = fold_superseded_reads(&mut self.transcript[..round]);
        if released > 0 {
            self.cache.note_compaction();
        }
        let sha = facts.sha.clone();
        if self.estimated_transcript_tokens() < min {
            let now = self.estimated_transcript_tokens();
            self.presenter
                .emit(forge_types::PresenterEvent::Warning(format!(
                    "checkpoint: folded the superseded reads behind {sha} ({} -> {})",
                    k(before_tokens),
                    k(now)
                )));
            return;
        }
        if let Err(error) = self.fold_slice(round, &landed, facts).await {
            tracing::warn!(%error, "checkpoint fold failed");
            return;
        }
        let after_tokens = self.estimated_transcript_tokens();
        self.presenter
            .emit(forge_types::PresenterEvent::Warning(format!(
                "checkpoint: folded the slice behind {sha} ({} -> {})",
                k(before_tokens),
                k(after_tokens)
            )));
    }

    async fn fold_slice(
        &mut self,
        round: usize,
        landed: &Landed,
        facts: CommitFacts,
    ) -> Result<(), crate::CoreError> {
        let before = self.transcript.len();
        let trigger = self.effective_compaction_trigger(None);
        let budget = CompactionBudget::for_trigger(trigger, self.pinned_tokens())
            .with_summary_cap(self.summary_cap());
        let source = self.store.compaction_source(&self.id).unwrap_or_default();
        let mut folded = fold_inputs(&self.transcript[..round], &source.tool_results);
        let kept_tail = self.transcript.len() - round;
        if let Ok(rows) = self.store.fold_preview(&self.id, kept_tail) {
            folded.new_episodes = crate::compaction_episodes::episodes_from_fold(&rows);
        }
        let previous = folded
            .messages
            .iter()
            .find(|m| m.role == Role::System && m.content.starts_with(SUMMARY_PREFIX))
            .map(|m| m.content.clone());
        let root = self.workspace_root().to_path_buf();
        let files = landed
            .edited
            .iter()
            .map(|path| (path.clone(), file_purpose(&root, path)))
            .collect();
        let open_tasks = self
            .tasks()
            .iter()
            .filter(|t| t.status != forge_types::TodoStatus::Done)
            .map(|t| {
                let state = match t.status {
                    forge_types::TodoStatus::InProgress => "in progress",
                    _ => "pending",
                };
                format!("[{state}] {}", t.title)
            })
            .collect();
        let mut summary = render_summary(&Facts {
            sha: &facts.sha,
            subject: &facts.subject,
            body: &facts.body,
            diffstat: &facts.diffstat,
            files,
            checks: &landed.checks,
            open_tasks,
            previous: previous.as_deref(),
        });
        let episodes = crate::compaction_episodes::render_episodes(
            &folded.previous_episodes,
            &folded.new_episodes,
        );
        let block = render_verbatim_block_within(
            &folded.previous_verbatim,
            &folded.user_messages,
            budget.verbatim_tokens,
            budget.verbatim_older_chars(),
        );
        for section in [episodes, block] {
            if !section.is_empty() {
                summary.push_str("\n\n");
                summary.push_str(&section);
            }
        }
        self.install_fold(
            crate::compaction_run::FoldInstall {
                split: round,
                summary: &summary,
                before,
                auto: true,
                note: None,
                restore_files: false,
            },
            &budget,
        )
        .await
        .map(|_| ())
    }
}
