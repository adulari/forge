//! `/star`, `/done` and `/archived`: the same session lifecycle the app sets, written straight to
//! the store without waiting for the session lock.

use forge_tui::{App, Deferred};

use super::{session_store, SharedSession};

#[derive(Clone, Copy)]
pub(super) enum Flag {
    Star,
    Done,
}

pub(super) fn toggle(session: &SharedSession, app: &mut App, flag: Flag) {
    let Some((store, id)) = session_store(session, &app.session_id) else {
        app.note("not ready for this turn yet; try again in a moment");
        return;
    };
    let inbox = app.deferred_inbox();
    tokio::task::spawn_blocking(move || {
        let note = match flip(&store, &id, flag) {
            Ok(note) => note,
            Err(e) => format!("⚠ {e}"),
        };
        inbox.post(Deferred::Notes(vec![note]));
    });
}

fn flip(
    store: &forge_store::Store,
    id: &str,
    flag: Flag,
) -> Result<String, forge_store::StoreError> {
    let (starred, done_at) = store
        .session_lifecycle(&[id.to_string()])?
        .remove(id)
        .unwrap_or((false, None));
    Ok(match flag {
        Flag::Star => {
            store.set_session_starred(id, !starred)?;
            if starred {
                "✓ session unstarred".into()
            } else {
                "✓ session starred — it floats in lists and skips auto-archive".into()
            }
        }
        Flag::Done => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
            store.set_session_done_at(id, if done_at.is_some() { None } else { Some(now) })?;
            if done_at.is_some() {
                "✓ session reopened".into()
            } else {
                "✓ session marked done".into()
            }
        }
    })
}

pub(super) fn archived(session: &SharedSession, app: &mut App, arg: &str) {
    let Some((store, _)) = session_store(session, &app.session_id) else {
        app.note("not ready for this turn yet; try again in a moment");
        return;
    };
    let arg = arg.trim().to_string();
    let inbox = app.deferred_inbox();
    tokio::task::spawn_blocking(move || {
        inbox.post(Deferred::Notes(vec![archived_note(&store, &arg)]));
    });
}

fn archived_note(store: &forge_store::Store, arg: &str) -> String {
    let rows = match store.list_sessions_for_resume() {
        Ok(rows) => rows.into_iter().filter(|s| s.archived).collect::<Vec<_>>(),
        Err(e) => return format!("⚠ /archived: {e}"),
    };
    if let Some(prefix) = arg.strip_prefix("restore").map(str::trim) {
        if prefix.is_empty() {
            return "usage: /archived restore <id>".into();
        }
        let matches: Vec<_> = rows.iter().filter(|s| s.id.starts_with(prefix)).collect();
        return match matches.as_slice() {
            [one] => match store.unarchive_session(&one.id) {
                Ok(()) => format!(
                    "✓ restored {} — /resume {} to open it",
                    short(&one.id),
                    short(&one.id)
                ),
                Err(e) => format!("⚠ /archived: {e}"),
            },
            [] => format!("no archived session matching '{prefix}'"),
            _ => format!("'{prefix}' matches several archived sessions; give more of the id"),
        };
    }
    if rows.is_empty() {
        return "no archived sessions".into();
    }
    let mut lines = vec![format!(
        "{} archived — /archived restore <id> brings one back",
        rows.len()
    )];
    for s in rows.iter().take(15) {
        let title = s
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| s.preview.clone())
            .unwrap_or_default();
        lines.push(format!(
            "  {}  {}",
            short(&s.id),
            title.chars().take(70).collect::<String>()
        ));
    }
    lines.join("\n")
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archived_session(store: &forge_store::Store) -> String {
        let id = store.create_session("/w", "default").unwrap();
        store
            .add_message(&id, 0, forge_types::Role::User, "fix the parser", None)
            .unwrap();
        store.archive_session(&id).unwrap();
        id
    }

    #[test]
    fn star_and_done_toggle() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let id = store.create_session("/w", "default").unwrap();
        assert!(flip(&store, &id, Flag::Star).unwrap().contains("starred"));
        assert!(flip(&store, &id, Flag::Star).unwrap().contains("unstarred"));
        assert!(flip(&store, &id, Flag::Done).unwrap().contains("done"));
        assert!(
            store.session_lifecycle(std::slice::from_ref(&id)).unwrap()[&id]
                .1
                .is_some()
        );
        assert!(flip(&store, &id, Flag::Done).unwrap().contains("reopened"));
    }

    #[test]
    fn archived_lists_and_restores_by_prefix() {
        let store = forge_store::Store::open_in_memory().unwrap();
        let id = archived_session(&store);
        assert!(archived_note(&store, "").contains("fix the parser"));
        assert!(archived_note(&store, "restore zzzz").contains("no archived session"));
        assert!(archived_note(&store, &format!("restore {}", &id[..6])).contains("restored"));
        assert!(!store.session_archived(&id).unwrap());
        assert_eq!(archived_note(&store, ""), "no archived sessions");
    }
}
