//! `forge import claude-session`: move a Claude Code session into Forge, history and all.
//!
//! The active chain is stored as a new Forge session. Everything Claude Code would no longer send
//! (before its last compaction) stays in history and replay but is marked compacted, exactly as
//! `/compact` does, so the next turn's model context is the compact summary plus what followed it.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use forge_store::{ImportedRow, ImportedSessionMeta};

use super::cc_transcript::Transcript;
use crate::*;

const BRIDGE_PREFIX: &str = "claude-cli::";
const TITLE_MAX_CHARS: usize = 80;

pub(crate) struct ClaudeSessionArgs {
    pub(crate) session: String,
    pub(crate) model: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) quiet: bool,
}

pub(crate) fn import_claude_session_cmd(args: ClaudeSessionArgs) -> Result<()> {
    let store = open_store()?;
    let started = std::time::Instant::now();
    let path = locate_transcript(&args.session)?;
    let report = import_transcript(&store, &path, &args)?;
    if !args.quiet {
        eprintln!(
            "imported {} ({} lines, {} compactions) in {:.1}s\n  {} messages stored, {} in the model context{}\n  model {}\n  open it with: forge chat --resume {}",
            path.display(),
            report.lines,
            report.compactions,
            started.elapsed().as_secs_f32(),
            report.stats.messages,
            report.stats.live,
            if report.stats.synthetic_results + report.stats.dropped_orphan_results > 0
                || report.skipped_lines > 0
            {
                format!(
                    " ({} placeholder results, {} orphan results dropped, {} unreadable lines skipped)",
                    report.stats.synthetic_results,
                    report.stats.dropped_orphan_results,
                    report.skipped_lines
                )
            } else {
                String::new()
            },
            report.model.as_deref().unwrap_or("(none pinned)"),
            report.session_id,
        );
    }
    println!("{}", report.session_id);
    Ok(())
}

pub(crate) struct ImportReport {
    pub(crate) session_id: String,
    pub(crate) lines: usize,
    pub(crate) skipped_lines: usize,
    pub(crate) compactions: usize,
    pub(crate) model: Option<String>,
    pub(crate) stats: super::cc_transcript::ImportStats,
}

/// A path to a `.jsonl`, or a Claude Code session id looked up under `~/.claude/projects`.
fn locate_transcript(session: &str) -> Result<PathBuf> {
    let direct = Path::new(session);
    if direct.is_file() {
        return Ok(direct.to_path_buf());
    }
    if session.contains(['/', '\\']) || session.ends_with(".jsonl") {
        bail!("no such transcript file: {session}");
    }
    let projects = forge_config::claude_dir()
        .context("no home directory — cannot locate ~/.claude")?
        .join("projects");
    let wanted = format!("{session}.jsonl");
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for dir in std::fs::read_dir(&projects)
        .with_context(|| format!("reading {}", projects.display()))?
        .flatten()
    {
        let candidate = dir.path().join(&wanted);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            found.push((meta.modified().unwrap_or(std::time::UNIX_EPOCH), candidate));
        }
    }
    found.sort();
    found.pop().map(|(_, p)| p).with_context(|| {
        format!(
            "no Claude Code session {session} under {}",
            projects.display()
        )
    })
}

fn pin_for(model: Option<&str>, transcript_model: Option<&str>) -> Option<String> {
    let raw = model.or(transcript_model)?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(if raw.contains("::") {
        raw.to_string()
    } else {
        format!("{BRIDGE_PREFIX}{raw}")
    })
}

fn clean_title(raw: &str) -> Option<String> {
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = one_line.chars().take(TITLE_MAX_CHARS).collect();
    Some(t).filter(|t| !t.is_empty())
}

pub(crate) fn import_transcript(
    store: &Store,
    path: &Path,
    args: &ClaudeSessionArgs,
) -> Result<ImportReport> {
    let transcript = Transcript::scan(path)?;
    if transcript.is_empty() {
        bail!(
            "{} holds no conversation (no user or assistant messages on its active chain)",
            path.display()
        );
    }
    let cwd = match (&args.cwd, &transcript.cwd) {
        (Some(c), _) => c.clone(),
        (None, Some(c)) => PathBuf::from(c),
        (None, None) => std::env::current_dir().context("no working directory")?,
    };
    let cwd = cwd.display().to_string();
    let title = args
        .title
        .as_deref()
        .and_then(clean_title)
        .or_else(|| transcript.title.as_deref().and_then(clean_title))
        .or_else(|| {
            transcript
                .first_prompt()
                .ok()
                .flatten()
                .as_deref()
                .and_then(clean_title)
        });
    let model = pin_for(args.model.as_deref(), transcript.model.as_deref());
    let summary = transcript.summary()?;

    let mut stats = Default::default();
    let mut failure: Option<anyhow::Error> = None;
    let stored = store.import_session_with(
        &ImportedSessionMeta {
            cwd: &cwd,
            mode: "default",
            title: title.as_deref(),
            pinned_model: model.as_deref(),
        },
        |sink| {
            let streamed = transcript.for_each_message(|m| {
                sink.push(ImportedRow {
                    role: m.role,
                    content: m.content,
                    model: m.model,
                    tool_calls: m.tool_calls,
                    tool_call_id: m.tool_call_id,
                    active: m.live,
                    created_at: m.timestamp,
                })?;
                Ok(())
            });
            match streamed {
                Ok(s) => {
                    stats = s;
                    Ok(summary)
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    failure = Some(e);
                    Err(forge_store::StoreError::InvalidValue(msg))
                }
            }
        },
    );
    let session_id = match stored {
        Ok(id) => id,
        Err(e) => return Err(failure.take().unwrap_or_else(|| e.into())),
    };
    Ok(ImportReport {
        session_id,
        lines: transcript.lines,
        skipped_lines: transcript.skipped_lines,
        compactions: transcript.compactions(),
        model,
        stats,
    })
}

#[cfg(test)]
mod tests;
