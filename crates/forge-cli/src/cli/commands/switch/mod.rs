//! `forge switch`: carry ONE conversation between Claude Code and Forge, either way, repeatedly.
//!
//! Each hop mirrors only what the other side has not seen (see [`sync`]) and then replaces this
//! process with the other tool, so the conversation simply continues.

pub(crate) mod cc_fs;
mod export;
mod launch;
mod sync;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use forge_store::{CcLink, SessionMeta, Store};

use self::launch::Launch;
use self::sync::Mirrored;
use super::cc_transcript::Transcript;
use crate::cli::args::switch::SwitchTarget;
use crate::cli::args::SwitchArgs;
use crate::*;

const TITLE_SHOWN_CHARS: usize = 60;
const DAEMON_PROBE: Duration = Duration::from_millis(800);

/// Where a conversation is being taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Claude { path: PathBuf, id: String },
    Forge { id: String },
}

impl Source {
    fn tool(&self) -> SwitchTarget {
        match self {
            Source::Claude { .. } => SwitchTarget::Claude,
            Source::Forge { .. } => SwitchTarget::Forge,
        }
    }
}

fn other(t: SwitchTarget) -> SwitchTarget {
    match t {
        SwitchTarget::Claude => SwitchTarget::Forge,
        SwitchTarget::Forge => SwitchTarget::Claude,
    }
}

fn cc_source(path: PathBuf) -> Result<Source> {
    let id = cc_fs::session_id_of(&path).context("transcript has no session id")?;
    Ok(Source::Claude { path, id })
}

fn unique_forge_id(store: &Store, prefix: &str) -> Result<Option<String>> {
    let mut matches = store.matching_session_ids(prefix)?;
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        n => bail!("'{prefix}' is ambiguous ({n} Forge sessions match): use more characters"),
    }
}

/// For a linked pair, the side that should be carried over: the one with turns the other lacks;
/// when both (or neither) moved, the newer one, and with no difference at all the tool the last
/// switch opened (you are presumably still there).
fn pair_source(store: &Store, link: &CcLink, forge: &SessionMeta) -> Result<Source> {
    let path = PathBuf::from(&link.cc_path);
    let meta = std::fs::metadata(&path).ok();
    let cc_len = meta.as_ref().map_or(0, |m| m.len() as i64);
    let cc_dirty = cc_len > link.cc_synced_bytes;
    let forge_dirty = store.mirrorable_rows_after(&forge.id, link.forge_synced_seq)? > 0;
    let cc_modified = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let claude_side = cc_source(path)?;
    let forge_side = Source::Forge {
        id: forge.id.clone(),
    };
    Ok(match (cc_dirty, forge_dirty, link.last_target.as_deref()) {
        (true, false, _) => claude_side,
        (false, true, _) => forge_side,
        (false, false, Some("claude")) => claude_side,
        (false, false, Some("forge")) => forge_side,
        _ if cc_modified > forge.last_activity => claude_side,
        _ => forge_side,
    })
}

/// Work out where the conversation comes from and which way it goes.
fn resolve(
    store: &Store,
    args: &SwitchArgs,
    cwd: &str,
    projects: &Path,
) -> Result<(Source, SwitchTarget)> {
    let source = match &args.session {
        Some(arg) => resolve_named(store, arg)?,
        None => resolve_here(store, projects, cwd, args.to)?,
    };
    let target = args.to.unwrap_or_else(|| other(source.tool()));
    if target == source.tool() {
        let link = linked_pair(store, &source)?;
        let Some((link, forge)) = link else {
            bail!(
                "that is already a {} session; use --to {} to move it the other way",
                if target == SwitchTarget::Claude {
                    "Claude Code"
                } else {
                    "Forge"
                },
                if target == SwitchTarget::Claude {
                    "forge"
                } else {
                    "claude"
                },
            );
        };
        // `--to` names the destination: the other half of the pair is the source.
        let source = match source {
            Source::Claude { .. } => Source::Forge { id: forge.id },
            Source::Forge { .. } => cc_source(PathBuf::from(link.cc_path))?,
        };
        return Ok((source, target));
    }
    Ok((source, target))
}

fn linked_pair(store: &Store, source: &Source) -> Result<Option<(CcLink, SessionMeta)>> {
    let link = match source {
        Source::Claude { id, .. } => store.cc_link_for_cc(id)?,
        Source::Forge { id } => store.cc_link_for_forge(id)?,
    };
    let Some(link) = link else { return Ok(None) };
    let forge = store.session_meta(&link.forge_session_id)?;
    Ok(forge.map(|f| (link, f)))
}

fn resolve_named(store: &Store, arg: &str) -> Result<Source> {
    if let Some(id) = unique_forge_id(store, arg)? {
        return Ok(Source::Forge { id });
    }
    let path = if std::path::Path::new(arg).is_file() {
        Some(PathBuf::from(arg))
    } else if arg.contains(['/', '\\']) || arg.ends_with(".jsonl") {
        None
    } else {
        cc_fs::find_by_id(arg)
    };
    match path {
        Some(p) => cc_source(std::fs::canonicalize(&p).unwrap_or(p)),
        None => bail!("no Forge session or Claude Code session matches '{arg}'"),
    }
}

fn resolve_here(
    store: &Store,
    projects: &Path,
    cwd: &str,
    to: Option<SwitchTarget>,
) -> Result<Source> {
    let cc = cc_fs::interactive_sessions(projects, cwd)
        .into_iter()
        .next();
    let forge = store.latest_session_for_cwd(cwd)?;
    let cc_src = cc.as_ref().map(|c| cc_source(c.path.clone())).transpose()?;
    let forge_src = forge.as_ref().map(|f| Source::Forge { id: f.id.clone() });
    match to {
        Some(SwitchTarget::Claude) => forge_src.with_context(|| format!("no Forge session for {cwd}")),
        Some(SwitchTarget::Forge) => cc_src.with_context(|| format!("no Claude Code session for {cwd}")),
        None => match (cc, forge) {
            (None, None) => bail!(
                "no Claude Code or Forge session found for {cwd}: run `forge switch` in a directory you have used one of them in"
            ),
            (Some(_), None) => Ok(cc_src.expect("cc present")),
            (None, Some(_)) => Ok(forge_src.expect("forge present")),
            (Some(c), Some(f)) => {
                let pair = store
                    .cc_link_for_cc(&c.id)?
                    .filter(|l| l.forge_session_id == f.id);
                if let Some(link) = pair {
                    return pair_source(store, &link, &f);
                }
                let cc_secs = c
                    .modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                Ok(if cc_secs > f.last_activity {
                    cc_src.expect("cc present")
                } else {
                    forge_src.expect("forge present")
                })
            }
        },
    }
}

/// `"title" (N messages, active 3m ago)` for the one-line summary.
fn describe(store: &Store, source: &Source) -> Result<String> {
    let (title, count, age) = match source {
        Source::Forge { id } => {
            let m = store.session_meta(id)?.context("session vanished")?;
            (display_title(&m), m.message_count as usize, m.last_activity)
        }
        Source::Claude { path, id } => {
            let mtime = std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            match store
                .cc_link_for_cc(id)?
                .and_then(|l| store.session_meta(&l.forge_session_id).ok().flatten())
            {
                Some(m) => (display_title(&m), m.message_count as usize, mtime),
                None => {
                    let t = Transcript::scan(path)?;
                    let title = t
                        .title
                        .clone()
                        .or_else(|| t.first_prompt().ok().flatten())
                        .unwrap_or_else(|| id.clone());
                    (one_line(&title), t.message_count(), mtime)
                }
            }
        }
    };
    Ok(format!(
        "\"{title}\" ({count} message{}, active {})",
        if count == 1 { "" } else { "s" },
        fmt_age(age)
    ))
}

fn one_line(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > TITLE_SHOWN_CHARS {
        let cut: String = flat.chars().take(TITLE_SHOWN_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        flat
    }
}

fn display_title(m: &SessionMeta) -> String {
    one_line(m.title.as_deref().or(m.preview.as_deref()).unwrap_or(&m.id))
}

async fn daemon_busy(forge_id: &str) -> bool {
    let Ok(token) = crate::serve::read_daemon_token() else {
        return false;
    };
    let Ok(http) = reqwest::Client::builder().timeout(DAEMON_PROBE).build() else {
        return false;
    };
    let base = crate::attach::resolve_base_url(None);
    crate::attach::fetch_sessions(&http, &base, &token)
        .await
        .is_ok_and(|list| {
            list.iter()
                .any(|s| s.id == forge_id && (s.busy || s.waiting))
        })
}

/// Reasons not to touch the sessions involved right now.
async fn blockers(store: &Store, source: &Source) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let (forge_id, cc) = match source {
        Source::Forge { id } => (
            Some(id.clone()),
            store
                .cc_link_for_forge(id)?
                .map(|l| (l.cc_session_id, PathBuf::from(l.cc_path))),
        ),
        Source::Claude { path, id } => (
            store.cc_link_for_cc(id)?.map(|l| l.forge_session_id),
            Some((id.clone(), path.clone())),
        ),
    };
    if let Some(fid) = forge_id {
        if daemon_busy(&fid).await {
            out.push(format!(
                "Forge session {fid} is busy in the forge serve daemon"
            ));
        } else if store.is_session_local_read_only(&fid).unwrap_or(false) {
            out.push(format!("Forge session {fid} is open in a terminal"));
        }
    }
    if let Some((id, path)) = cc {
        let cwd = path_cwd(store, &path, &id);
        if let Some(why) = cc_fs::open_reason(&id, &path, &cwd) {
            out.push(format!("Claude Code session {id} looks open: {why}"));
        }
    }
    Ok(out)
}

fn path_cwd(store: &Store, path: &Path, cc_id: &str) -> String {
    store
        .cc_link_for_cc(cc_id)
        .ok()
        .flatten()
        .and_then(|l| store.session_meta(&l.forge_session_id).ok().flatten())
        .map(|m| m.cwd)
        .or_else(|| path.parent().map(|p| p.display().to_string()))
        .unwrap_or_default()
}

fn claude_launch(store: &Store, forge_id: &str, cc_id: &str) -> Result<(Launch, Option<String>)> {
    let meta = store.session_meta(forge_id)?.context("session vanished")?;
    let pinned = meta.pinned_model.as_deref();
    let (model, note) = match pinned {
        Some(p) => match p.strip_prefix("claude-cli::") {
            Some(bare) => (Some(bare.to_string()), None),
            None => (
                None,
                Some(format!(
                    "note: pinned to {p}; Claude Code will use its default model"
                )),
            ),
        },
        None => (None, None),
    };
    Ok((
        Launch::claude(cc_id, model.as_deref(), &meta.permission_mode, &meta.cwd),
        note,
    ))
}

static REQUESTED: Mutex<Option<String>> = Mutex::new(None);

/// `/claude` in the TUI: when there is something to carry over, remember the session so the TUI
/// can leave the terminal cleanly first (see [`take_requested`]); otherwise say why not.
pub(crate) fn stage_claude_switch(forge_id: &str, in_terminal: bool) -> Result<(), &'static str> {
    if !in_terminal {
        return Err("⏻ /claude needs a terminal session");
    }
    let empty = open_store()
        .and_then(|s| Ok(s.mirrorable_rows_after(forge_id, -1)?))
        .is_ok_and(|n| n == 0);
    if empty {
        return Err("⏻ nothing to carry over yet: send a message first");
    }
    if let Ok(mut r) = REQUESTED.lock() {
        *r = Some(forge_id.to_string());
    }
    Ok(())
}

/// The session the TUI asked to move to Claude Code on its way out, if any.
pub(crate) fn take_requested() -> Option<String> {
    REQUESTED.lock().ok().and_then(|mut r| r.take())
}

/// Move one Forge session to Claude Code (what `/claude` does once the TUI has closed).
pub(crate) async fn switch_forge_session_to_claude(forge_id: &str) -> Result<()> {
    switch_cmd(SwitchArgs {
        session: Some(forge_id.to_string()),
        to: Some(SwitchTarget::Claude),
        dry_run: false,
        no_launch: false,
        force: false,
    })
    .await
}

fn finish(launch: Launch, no_launch: bool) -> Result<()> {
    if no_launch {
        println!("run: {}", launch.display());
        return Ok(());
    }
    launch.run()
}

pub(crate) async fn switch_cmd(args: SwitchArgs) -> Result<()> {
    let store = open_store()?;
    let cwd = std::env::current_dir()
        .context("no working directory")?
        .display()
        .to_string();
    let (source, target) = resolve(&store, &args, &cwd, &cc_fs::projects_dir()?)?;
    let from = if source.tool() == SwitchTarget::Claude {
        "Claude Code"
    } else {
        "Forge"
    };
    let to = if target == SwitchTarget::Claude {
        "Claude Code"
    } else {
        "Forge"
    };
    println!("{from} {} → {to}", describe(&store, &source)?);

    let found = blockers(&store, &source).await?;
    if !found.is_empty() && !args.force {
        let list = found.join("; ");
        if args.dry_run {
            println!("dry run: would refuse ({list}); --force overrides");
            return Ok(());
        }
        bail!("not switching: {list}. Close it there, or pass --force");
    }

    match &source {
        Source::Forge { id } => {
            let meta = store.session_meta(id)?.context("session vanished")?;
            if args.dry_run {
                let linked = store.cc_link_for_forge(id)?;
                println!(
                    "dry run: would {} then run `claude --resume …` in {}",
                    match linked {
                        Some(l) => format!(
                            "append new Forge turns to Claude Code session {}",
                            l.cc_session_id
                        ),
                        None => "create a new Claude Code session from it".to_string(),
                    },
                    meta.cwd
                );
                return Ok(());
            }
            let report = sync::export_to_claude(&store, id)?;
            if let Some(w) = &report.warning {
                eprintln!("warning: {w}");
            }
            let (launch, note) = claude_launch(&store, id, &report.cc_session_id)?;
            if let Some(n) = note {
                eprintln!("{n}");
            }
            finish(launch, args.no_launch)
        }
        Source::Claude { path, id } => {
            let mode = format!(
                "{:?}",
                forge_config::load()
                    .map(|c| c.permission_mode)
                    .unwrap_or(forge_types::PermissionMode::Default)
            );
            if args.dry_run {
                let linked = store.cc_link_for_cc(id)?;
                println!(
                    "dry run: would {}, then run `forge chat --resume …`",
                    match linked {
                        Some(l) => format!(
                            "mirror new Claude Code turns into Forge session {}",
                            l.forge_session_id
                        ),
                        None => "import it as a new Forge session".to_string(),
                    }
                );
                return Ok(());
            }
            let report = sync::mirror_from_claude(&store, path, &mode)?;
            if let Some(w) = &report.warning {
                eprintln!("warning: {w}");
            }
            let _: &Mirrored = &report.kind;
            let meta = store
                .session_meta(&report.forge_id)?
                .context("session vanished")?;
            let cc_mode = cc_fs::last_permission_mode(path);
            finish(
                Launch::forge(&report.forge_id, cc_mode.as_deref(), &meta.cwd)?,
                args.no_launch,
            )
        }
    }
}
