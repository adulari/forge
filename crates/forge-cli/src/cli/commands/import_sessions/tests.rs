use std::io::Write;

use serde_json::json;

use super::*;
use crate::cli::commands::codex_rollout::tests::{write_rollout, CODEX_ID};

fn write_claude(projects: &Path, cwd: &str, id: &str) -> PathBuf {
    let dir = projects.join("-work-proj");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.jsonl"));
    let mut f = std::fs::File::create(&path).unwrap();
    let lines = [
        json!({"type": "user", "uuid": "u1", "parentUuid": null, "cwd": cwd, "entrypoint": "cli",
            "timestamp": "2026-10-02T10:00:00Z",
            "message": {"role": "user", "content": "fix the flaky test"}}),
        json!({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "timestamp": "2026-10-02T10:00:01Z",
            "message": {"id": "m1", "role": "assistant", "model": "claude-opus-5-5",
                "content": [{"type": "text", "text": "Fixed."}]}}),
    ];
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    path
}

struct Fixture {
    _dir: tempfile::TempDir,
    projects: PathBuf,
    sessions: PathBuf,
    store_dir: tempfile::TempDir,
}

fn fixture(cwd: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("claude").join("projects");
    let sessions = dir.path().join("codex").join("sessions");
    write_claude(&projects, cwd, "c0ffee00-0000-4000-8000-000000000001");
    write_rollout(&sessions, cwd);
    Fixture {
        _dir: dir,
        projects,
        sessions,
        store_dir: tempfile::tempdir().unwrap(),
    }
}

fn picks(found: &[Candidate]) -> Vec<Selection> {
    found
        .iter()
        .map(|c| Selection {
            agent: c.agent,
            id: c.id.clone(),
        })
        .collect()
}

#[test]
fn a_scan_lists_both_agents_newest_first_with_titles_and_directories() {
    let cwd = tempfile::tempdir().unwrap();
    let fx = fixture(&cwd.path().display().to_string());
    let found = scan_in(Some(&fx.projects), Some(&fx.sessions), None);
    assert_eq!(found.len(), 2);
    let claude = found.iter().find(|c| c.agent == Agent::Claude).unwrap();
    let codex = found.iter().find(|c| c.agent == Agent::Codex).unwrap();
    assert_eq!(claude.title, "fix the flaky test");
    assert_eq!(codex.title, "list the files and count them");
    assert_eq!(codex.id, CODEX_ID);
    assert!(claude.cwd_exists && codex.cwd_exists);
    assert!(claude.imported_session_id.is_none());
}

#[test]
fn importing_both_makes_resumable_sessions_and_a_rescan_marks_them() {
    let cwd = tempfile::tempdir().unwrap();
    let fx = fixture(&cwd.path().display().to_string());
    let store = Store::open(fx.store_dir.path().join("forge.db")).unwrap();
    let found = scan_in(Some(&fx.projects), Some(&fx.sessions), Some(&store));
    let outcomes = import_from(&store, &found, &picks(&found), None, "Default");
    assert!(
        outcomes.iter().all(|o| o.error.is_none() && !o.skipped),
        "{outcomes:?}"
    );

    for o in &outcomes {
        let id = o.session_id.as_deref().unwrap();
        let msgs = store.load_messages(id).unwrap();
        assert!(!msgs.is_empty());
        assert!(msgs.iter().any(|m| m.role == Role::User), "{:?}", o.agent);
        let listed = store.list_sessions().unwrap();
        assert!(listed
            .iter()
            .any(|s| s.id == id && s.cwd == cwd.path().display().to_string()));
    }
    let codex = outcomes.iter().find(|o| o.agent == Agent::Codex).unwrap();
    assert_eq!(
        store
            .session_pinned_model(codex.session_id.as_deref().unwrap())
            .unwrap()
            .as_deref(),
        Some("codex-cli::gpt-5-codex")
    );

    let again = scan_in(Some(&fx.projects), Some(&fx.sessions), Some(&store));
    assert!(
        again.iter().all(|c| c.imported_session_id.is_some()),
        "{again:?}"
    );
    let twice = import_from(&store, &again, &picks(&again), None, "Default");
    assert!(twice.iter().all(|o| o.skipped), "no duplicates");
    assert_eq!(store.list_sessions().unwrap().len(), 2);
}

#[test]
fn a_missing_directory_falls_back_to_the_chosen_project() {
    let fx = fixture("/no/such/dir/anywhere");
    let store = Store::open(fx.store_dir.path().join("forge.db")).unwrap();
    let found = scan_in(Some(&fx.projects), Some(&fx.sessions), Some(&store));
    assert!(found.iter().all(|c| !c.cwd_exists));
    let fallback = tempfile::tempdir().unwrap();
    let outcomes = import_from(
        &store,
        &found,
        &picks(&found),
        Some(fallback.path()),
        "Default",
    );
    assert!(outcomes.iter().all(|o| o.error.is_none()), "{outcomes:?}");
    for s in store.list_sessions().unwrap() {
        assert_eq!(s.cwd, fallback.path().display().to_string());
    }
}

#[test]
fn an_unknown_pick_fails_alone_and_a_forgotten_import_can_be_redone() {
    let cwd = tempfile::tempdir().unwrap();
    let fx = fixture(&cwd.path().display().to_string());
    let store = Store::open(fx.store_dir.path().join("forge.db")).unwrap();
    let found = scan_in(Some(&fx.projects), Some(&fx.sessions), Some(&store));
    let mut sel = picks(&found);
    sel.push(Selection {
        agent: Agent::Codex,
        id: "nope".into(),
    });
    let outcomes = import_from(&store, &found, &sel, None, "Default");
    assert_eq!(outcomes.iter().filter(|o| o.error.is_some()).count(), 1);

    let codex = outcomes
        .iter()
        .find(|o| o.agent == Agent::Codex && o.error.is_none())
        .unwrap();
    store
        .delete_session_report(codex.session_id.as_deref().unwrap())
        .unwrap();
    let again = scan_in(Some(&fx.projects), Some(&fx.sessions), Some(&store));
    let c = again.iter().find(|c| c.agent == Agent::Codex).unwrap();
    assert!(
        c.imported_session_id.is_none(),
        "a deleted session is importable again"
    );
}

#[test]
fn sources_are_never_modified() {
    let cwd = tempfile::tempdir().unwrap();
    let fx = fixture(&cwd.path().display().to_string());
    let before: Vec<_> = [&fx.projects, &fx.sessions]
        .iter()
        .flat_map(|d| walk(d))
        .map(|p| (p.clone(), std::fs::read(&p).unwrap()))
        .collect();
    let store = Store::open(fx.store_dir.path().join("forge.db")).unwrap();
    let found = scan_in(Some(&fx.projects), Some(&fx.sessions), Some(&store));
    import_from(&store, &found, &picks(&found), None, "Default");
    for (p, bytes) in before {
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "{}", p.display());
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push(e.path());
        }
    }
    out
}
