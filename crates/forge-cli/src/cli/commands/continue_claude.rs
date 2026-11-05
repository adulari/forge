//! Bring one Claude Code conversation into Forge: import it the first time, afterwards copy only
//! the turns Claude Code added. The same two paths `forge switch` and the daemon's
//! `POST /api/claude/sessions/{id}/continue` take, for callers that hold a store in-process (the
//! TUI's `/continue-claude` and `/import`).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use forge_store::Store;

use super::import_sessions::{self, Agent, Selection};
use super::switch::sync::{self, Mirrored};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Continued {
    pub(crate) session_id: String,
    /// True the first time (a new Forge session), false when an existing one was synced.
    pub(crate) imported: bool,
    pub(crate) appended: usize,
}

pub(crate) fn continue_claude_session(
    store: &Store,
    cc_id: &str,
    fallback_cwd: Option<&Path>,
    mode: &str,
) -> Result<Continued> {
    let candidates = import_sessions::scan(Some(store));
    let found = candidates
        .iter()
        .find(|c| c.agent == Agent::Claude && c.id == cc_id)
        .ok_or_else(|| anyhow!("no Claude Code session {cc_id} on this machine"))?;
    let link = store
        .cc_link_for_cc(cc_id)?
        .filter(|l| store.session_exists(&l.forge_session_id).unwrap_or(false));
    let Some(link) = link else {
        let pick = Selection {
            agent: Agent::Claude,
            id: cc_id.to_string(),
        };
        let outcome =
            import_sessions::import_from(store, &candidates, &[pick], fallback_cwd, mode)
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("nothing imported"))?;
        if let Some(error) = outcome.error {
            bail!(error);
        }
        let session_id = outcome
            .session_id
            .ok_or_else(|| anyhow!("import produced no session"))?;
        return Ok(Continued {
            session_id,
            imported: true,
            appended: outcome.messages,
        });
    };
    let own = PathBuf::from(&link.cc_path);
    let path = if own.is_file() { own } else { found.path.clone() };
    let report = sync::mirror_from_claude(store, &path, mode)?;
    let (imported, appended) = match report.kind {
        Mirrored::Imported | Mirrored::Reimported => (true, 0),
        Mirrored::Appended(n) => (false, n),
        Mirrored::Unchanged => (false, 0),
    };
    Ok(Continued {
        session_id: report.forge_id,
        imported,
        appended,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_claude_session_is_an_error() {
        let store = Store::open_in_memory().unwrap();
        let err = continue_claude_session(&store, "no-such-session-id", None, "Default");
        assert!(err.is_err());
    }
}
