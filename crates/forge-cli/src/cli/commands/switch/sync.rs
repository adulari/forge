//! Moving the new part of one conversation across: Forge -> Claude Code and back.
//!
//! The link row (`cc_session_link`) records how far each side has been mirrored, so a second
//! switch with nothing new adds nothing and a long conversation costs only its new turns.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use forge_store::{CcLink, CcLinkUpdate, ImportedRow, Store};

use super::cc_fs;
use super::export::{build_entries, ExportContext, Exported};
use crate::cli::commands::cc_transcript::Transcript;
use crate::cli::commands::import_claude_session::{import_transcript, ClaudeSessionArgs};

const TITLE_MAX_CHARS: usize = 80;

#[derive(Debug)]
pub(crate) struct ExportReport {
    pub(crate) cc_session_id: String,
    pub(crate) warning: Option<String>,
}

fn title_for(meta: &forge_store::SessionMeta) -> Option<String> {
    let raw = meta.title.as_deref().or(meta.preview.as_deref())?;
    let one_line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(one_line.chars().take(TITLE_MAX_CHARS).collect::<String>()).filter(|t| !t.is_empty())
}

fn fsync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Write a new transcript: a temp file in the same directory, synced, then renamed, so Claude
/// Code can never see half of it.
fn write_new_file(dir: &Path, id: &str, bytes: &[u8]) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".{id}.jsonl.tmp"));
    let final_path = dir.join(format!("{id}.jsonl"));
    let result = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &final_path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", final_path.display()));
    }
    fsync_dir(dir);
    Ok(final_path)
}

/// Append complete lines to an existing transcript with one write, then sync. Returns the new
/// length.
fn append_lines(path: &Path, bytes: &[u8]) -> Result<u64> {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    let len = f.metadata()?.len();
    let mut buf = Vec::with_capacity(bytes.len() + 1);
    if len > 0 {
        f.seek(SeekFrom::Start(len - 1))?;
        let mut last = [0u8; 1];
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            buf.push(b'\n');
        }
    }
    buf.extend_from_slice(bytes);
    f.write_all(&buf)?;
    f.sync_all()?;
    Ok(f.metadata()?.len())
}

/// Forge -> Claude Code: create or extend the linked transcript so it holds everything Forge's
/// model would see next. Nothing is written when there is nothing new.
pub(crate) fn export_to_claude(store: &Store, forge_id: &str) -> Result<ExportReport> {
    export_to_claude_in(
        store,
        forge_id,
        &cc_fs::projects_dir()?,
        &cc_fs::claude_version(),
    )
}

/// [`export_to_claude`] against an explicit projects directory and Claude Code version.
pub(crate) fn export_to_claude_in(
    store: &Store,
    forge_id: &str,
    projects: &Path,
    version: &str,
) -> Result<ExportReport> {
    let meta = store
        .session_meta(forge_id)?
        .with_context(|| format!("no Forge session {forge_id}"))?;
    let branch = cc_fs::git_branch(&meta.cwd);
    let summary = store.session_compaction_summary(forge_id)?.map(|(s, _)| s);
    let link = store.cc_link_for_forge(forge_id)?;

    let Some(link) = link.filter(|l| Path::new(&l.cc_path).is_file()) else {
        return export_new(
            store,
            &meta,
            projects,
            version,
            branch.as_deref(),
            summary.as_deref(),
        );
    };

    let path = PathBuf::from(&link.cc_path);
    let len = std::fs::metadata(&path)?.len();
    let compaction_changed = summary.is_some() && summary != link.forge_summary;
    let after = if compaction_changed {
        -1
    } else {
        link.forge_synced_seq
    };
    let rows = store.export_rows_after(forge_id, after)?;
    let max_seq = rows
        .last()
        .map_or(link.forge_synced_seq, |r| r.seq.max(link.forge_synced_seq));

    let mut leaf = link.cc_last_uuid.clone();
    let mut cc_gained = false;
    if len > link.cc_synced_bytes as u64 {
        let tail = Transcript::scan_from(&path, link.cc_synced_bytes as u64)?;
        if tail.last_uuid.is_some() {
            leaf = tail.last_uuid.clone();
            cc_gained = !tail.is_empty();
        }
    }
    let ctx = ExportContext {
        cc_session_id: &link.cc_session_id,
        cwd: &meta.cwd,
        version,
        git_branch: branch.as_deref(),
    };
    let exported = build_entries(
        &ctx,
        leaf.as_deref(),
        None,
        compaction_changed.then_some(summary.as_deref()).flatten(),
        &rows,
    );
    if exported.lines.is_empty() {
        store.write_cc_link(&CcLink {
            last_target: Some("claude".into()),
            ..link.clone()
        })?;
        return Ok(ExportReport {
            cc_session_id: link.cc_session_id,
            warning: None,
        });
    }
    let warning = cc_gained.then(|| {
        "both sides changed since the last switch: Claude Code's newer entries stay on its side; Forge's new turns follow them".to_string()
    });
    let new_len = append_lines(&path, &exported.bytes())?;
    store.write_cc_link(&CcLink {
        cc_synced_bytes: new_len as i64,
        cc_last_uuid: exported.last_uuid.clone().or(link.cc_last_uuid.clone()),
        forge_synced_seq: max_seq,
        forge_summary: summary.or(link.forge_summary.clone()),
        last_target: Some("claude".into()),
        ..link.clone()
    })?;
    Ok(ExportReport {
        cc_session_id: link.cc_session_id,
        warning,
    })
}

fn export_new(
    store: &Store,
    meta: &forge_store::SessionMeta,
    projects: &Path,
    version: &str,
    branch: Option<&str>,
    summary: Option<&str>,
) -> Result<ExportReport> {
    let rows = store.export_rows_after(&meta.id, -1)?;
    let max_seq = rows.last().map_or(-1, |r| r.seq);
    let cc_id = forge_types::new_id();
    let ctx = ExportContext {
        cc_session_id: &cc_id,
        cwd: &meta.cwd,
        version,
        git_branch: branch,
    };
    let title = title_for(meta);
    let Exported { lines, last_uuid } = build_entries(&ctx, None, title.as_deref(), summary, &rows);
    if last_uuid.is_none() {
        bail!(
            "session {} has no conversation Claude Code could continue",
            meta.id
        );
    }
    let dir = cc_fs::project_dir(projects, &meta.cwd)?;
    let bytes = lines.concat().into_bytes();
    let path = write_new_file(&dir, &cc_id, &bytes)?;
    store.write_cc_link(&CcLink {
        forge_session_id: meta.id.clone(),
        cc_session_id: cc_id.clone(),
        cc_path: path.display().to_string(),
        cc_synced_bytes: bytes.len() as i64,
        cc_last_uuid: last_uuid,
        forge_synced_seq: max_seq,
        forge_summary: summary.map(str::to_string),
        last_target: Some("claude".into()),
        updated_at: 0,
    })?;
    Ok(ExportReport {
        cc_session_id: cc_id,
        warning: None,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Mirrored {
    /// First time this Claude Code session came across: imported as a new Forge session.
    Imported,
    /// Linked; this many Forge rows were added.
    Appended(usize),
    Unchanged,
    /// The Claude Code side could not be followed incrementally; imported afresh.
    Reimported,
}

#[derive(Debug)]
pub(crate) struct MirrorReport {
    pub(crate) forge_id: String,
    pub(crate) kind: Mirrored,
    pub(crate) warning: Option<String>,
}

fn import_args(path: &Path) -> ClaudeSessionArgs {
    ClaudeSessionArgs {
        session: path.display().to_string(),
        model: None,
        title: None,
        cwd: None,
        quiet: true,
    }
}

/// Claude Code -> Forge: import the transcript the first time, afterwards read only the bytes
/// added since the last sync and append them to the linked Forge session.
pub(crate) fn mirror_from_claude(store: &Store, path: &Path, mode: &str) -> Result<MirrorReport> {
    let cc_id = super::cc_fs::session_id_of(path).context("transcript has no session id")?;
    let reimport = |why: &str| -> Result<MirrorReport> {
        let report = import_transcript(store, path, &import_args(path), mode)?;
        Ok(MirrorReport {
            forge_id: report.session_id,
            kind: Mirrored::Reimported,
            warning: Some(format!("{why}; imported it as a new Forge session")),
        })
    };
    let Some(link) = store.cc_link_for_cc(&cc_id)? else {
        let report = import_transcript(store, path, &import_args(path), mode)?;
        return Ok(MirrorReport {
            forge_id: report.session_id,
            kind: Mirrored::Imported,
            warning: None,
        });
    };
    let len = std::fs::metadata(path)?.len();
    let synced = link.cc_synced_bytes as u64;
    if len < synced {
        return reimport("the Claude Code transcript was rewritten");
    }
    let abs = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string();
    let tail = Transcript::scan_from(path, synced)?;
    let advance = CcLinkUpdate {
        cc_session_id: &cc_id,
        cc_path: &abs,
        cc_synced_bytes: tail.bytes_end as i64,
        cc_last_uuid: tail.last_uuid.as_deref(),
        last_target: Some("forge"),
    };
    if tail.last_uuid.is_none() {
        store.write_cc_link(&CcLink {
            cc_path: advance.cc_path.to_string(),
            cc_synced_bytes: advance.cc_synced_bytes,
            last_target: Some("forge".into()),
            ..link.clone()
        })?;
        return Ok(MirrorReport {
            forge_id: link.forge_session_id,
            kind: Mirrored::Unchanged,
            warning: None,
        });
    }
    if link.cc_last_uuid.is_some() && tail.root_parent != link.cc_last_uuid {
        return reimport("Claude Code rewound or branched this session");
    }
    if tail.has_unresolved_preserved_segment() {
        return reimport("Claude Code compacted this session and kept earlier turns verbatim");
    }
    let compact = tail.has_boundary();
    let summary = if compact { tail.summary()? } else { None };
    let forge_gained =
        store.mirrorable_rows_after(&link.forge_session_id, link.forge_synced_seq)? > 0;
    let mut pushed = 0usize;
    let mut failure: Option<anyhow::Error> = None;
    let updated = store.mirror_into_session_with(&link, &advance, compact, |sink| {
        let streamed = tail.for_each_message(|m| {
            sink.push(ImportedRow {
                role: m.role,
                content: m.content,
                model: m.model,
                tool_calls: m.tool_calls,
                tool_call_id: m.tool_call_id,
                active: m.live,
                created_at: m.timestamp,
            })?;
            pushed += 1;
            Ok(())
        });
        match streamed {
            Ok(_) => Ok(summary.clone()),
            Err(e) => {
                let msg = format!("{e:#}");
                failure = Some(e);
                Err(forge_store::StoreError::InvalidValue(msg))
            }
        }
    });
    let updated = match updated {
        Ok(l) => l,
        Err(e) => return Err(failure.take().unwrap_or_else(|| e.into())),
    };
    let warning = (forge_gained && pushed > 0).then(|| {
        "both sides changed since the last switch: Forge's newer turns stay on its side; Claude Code's new turns follow them".to_string()
    });
    Ok(MirrorReport {
        forge_id: updated.forge_session_id,
        kind: if pushed == 0 {
            Mirrored::Unchanged
        } else {
            Mirrored::Appended(pushed)
        },
        warning,
    })
}
