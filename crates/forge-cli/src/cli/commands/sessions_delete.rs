//! `forge sessions --delete <id>`: permanently delete one session after showing what goes.

use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use forge_store::SessionDeletePreview;

use super::replay::{open_store, resolve_session};
use crate::session_delete::{self, Refusal};

pub(crate) async fn sessions_delete_cmd(prefix: &str, yes: bool, compact_db: bool) -> Result<()> {
    let store = Arc::new(open_store()?);
    let id = resolve_session(&store, prefix)?;
    let preview = store
        .session_delete_preview(&id)?
        .context("session vanished while resolving it")?;
    let daemon = crate::serve::read_state()
        .ok()
        .flatten()
        .filter(|state| state.process_is_alive());
    print!("{}", summary(&preview));

    if compact_db {
        if let Err(why) = compact_refusal(&store, daemon.is_some()) {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
    // Without a daemon this process applies the checks itself; with one, the daemon's route does,
    // against the live registry only it can see. Local checks still run first so a refusal never
    // costs the person a confirmation.
    if let Err(refusal) = session_delete::check(&store, &id, false).await {
        if daemon.is_none() || !matches!(refusal, Refusal::NotFound) {
            return refused(&refusal);
        }
    }
    if !yes && !confirmed(&id)? {
        println!("cancelled, nothing was deleted");
        return Ok(());
    }

    let done = match session_delete::delete_anywhere(&store, &id).await {
        Ok(done) => done,
        Err(refusal) => return refused(&refusal),
    };
    let (sessions, files, scrubbed) = (done.sessions, done.files_removed, done.wal_truncated);
    println!(
        "deleted {sessions} session(s){}; database scrubbed ({})",
        if files > 0 {
            format!(" and {files} snapshot folder(s)")
        } else {
            String::new()
        },
        if scrubbed {
            "secure_delete, WAL truncated"
        } else {
            "secure_delete; WAL still held open by another process and clears at its next checkpoint"
        }
    );
    if compact_db {
        store.vacuum().context("compacting the database")?;
        println!("database file compacted (VACUUM): no trace of the session is left in it");
    } else {
        println!(
            "note: SQLite can leave fragments in pages it rebalanced; for a byte-level guarantee \
             run with --compact-db (daemon and chats closed)"
        );
    }
    Ok(())
}

/// A refusal is an answer, not a crash: print it with its next step and exit 1 without the
/// generic "run forge doctor" hint the top-level error printer would append.
fn refused(refusal: &Refusal) -> Result<()> {
    eprintln!("not deleted: {}", refusal.message());
    if !refusal.next_step().is_empty() {
        eprintln!("  next: {}", refusal.next_step());
    }
    std::process::exit(1)
}

/// VACUUM rewrites the whole file and needs every other connection out of the way.
fn compact_refusal(store: &forge_store::Store, daemon_running: bool) -> Result<()> {
    let terminals = store
        .local_live_sessions()
        .map(|rows| rows.len())
        .unwrap_or(0);
    if daemon_running || terminals > 0 {
        bail!(
            "not deleted: --compact-db needs exclusive access to the database, but {}.\n  next: stop the daemon \
             (`forge service stop`) and close every `forge chat`, then run this again; or leave \
             the flag off: the delete is already scrubbed without it",
            if daemon_running {
                "the Forge daemon is running"
            } else {
                "a `forge chat` terminal is open"
            }
        );
    }
    Ok(())
}

fn summary(p: &SessionDeletePreview) -> String {
    let title = p.title.as_deref().filter(|t| !t.trim().is_empty());
    let mut out = format!(
        "session {}  {}\n  {} turn(s), {} message(s), {} tool call(s)\n  {} subagent session(s) will be deleted with it\n  worktree: {}\n",
        &p.id[..p.id.len().min(8)],
        title.map_or_else(|| "(untitled)".to_string(), |t| format!("\"{t}\"")),
        p.turns,
        p.messages,
        p.tool_calls,
        p.children,
        p.worktree.as_deref().unwrap_or("none"),
    );
    if p.synced {
        out.push_str("  it is synced: it is removed from your other devices too\n");
    }
    if p.memories_kept > 0 {
        out.push_str(&format!(
            "  {} saved memory item(s) came from it and are kept (`forge memory` removes them)\n",
            p.memories_kept
        ));
    }
    out.push_str("This is permanent and cannot be undone.\n");
    out
}

fn confirmed(id: &str) -> Result<bool> {
    let short = &id[..id.len().min(8)];
    if !std::io::stdin().is_terminal() {
        bail!("refusing to delete without a terminal to confirm on; pass --yes to skip the prompt");
    }
    print!("Type {short} to delete it permanently: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim() == short)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_names_the_session_and_what_goes() {
        let preview = SessionDeletePreview {
            id: "0123456789abcdef".into(),
            title: Some("fix the parser".into()),
            turns: 4,
            messages: 9,
            tool_calls: 3,
            children: 2,
            worktree: Some("/tmp/wt".into()),
            synced: true,
            frozen: false,
            memories_kept: 1,
            targets: Vec::new(),
        };
        let text = summary(&preview);
        for needle in [
            "01234567",
            "\"fix the parser\"",
            "4 turn(s)",
            "2 subagent",
            "/tmp/wt",
            "other devices",
            "kept",
            "permanent",
        ] {
            assert!(text.contains(needle), "{needle} missing from:\n{text}");
        }
    }

    #[test]
    fn compact_is_refused_while_a_daemon_runs() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let err = compact_refusal(&store, true).unwrap_err().to_string();
        assert!(
            err.contains("daemon is running") && err.contains("next:"),
            "{err}"
        );
        assert!(compact_refusal(&store, false).is_ok());
    }
}
