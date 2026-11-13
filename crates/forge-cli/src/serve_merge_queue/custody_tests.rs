//! Custody of a PR when its goal changes workers (docs/features/merge-queue.md, "Relays").

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use super::tests::mock_state;
use super::tests::{
    arm_sessions, disarm, git, merged_order, resolve_four_by_hand, sweeps, watch_of, World,
};
use super::update::run_step;
use super::*;
use crate::serve::tests::FORGE_DB_LOCK;
use crate::serve::DaemonState;
use crate::serve_pr_watch::{registry, Phase};

pub(super) async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..300 {
        if ready() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn has_conflict_turn(state: &DaemonState, id: &str) -> bool {
    state
        .store
        .load_messages(id)
        .unwrap_or_default()
        .iter()
        .any(|m| {
            m.content
                .contains("Merge queue: this pull request is next to land")
        })
}

/// What the relay driver does: the store hand-off, then the daemon is asked to host the worker.
fn relay(state: &DaemonState, predecessor: &str, worktree: &Path) -> String {
    state
        .store
        .set_session_worktree(predecessor, &worktree.display().to_string())
        .unwrap();
    let successor = state
        .store
        .create_relay_successor(predecessor, "wedged", "same failing set", "BRIEF")
        .unwrap();
    assert!(state.registry.launch_relay(successor.clone()));
    successor
}

/// The head is in the queue but cannot land yet (its worktree has an uncommitted edit).
async fn park_head(state: &Arc<DaemonState>, w: &World, ids: &[String]) {
    std::fs::write(w.clones[0].join("shared.txt"), "edited\n").unwrap();
    sweeps(state, w, 2, 900).await;
    let head = watch_of(&ids[0]);
    assert!(head.queue.is_some() && head.active(), "{head:?}");
    assert_eq!(
        head.hold.as_deref(),
        Some("the worktree has uncommitted changes")
    );
}

async fn hosted(state: &Arc<DaemonState>, id: &str) -> bool {
    state.registry.get(id).await.is_some()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relayed_goal_takes_its_watch_and_the_fix_turn_and_the_queue_lands_in_order() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::build(
        &[("1", "from one"), ("4", "from four"), ("2", ""), ("3", "")],
        false,
    );
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    crate::serve_relay::start(&state);
    let ids = arm_sessions(&state, &w).await;
    sweeps(&state, &w, 1, 900).await;

    // PR4's goal moves to a fresh worker twice before the conflict is found.
    let second = relay(&state, &ids[1], &w.clones[1]);
    wait_for("the first fresh worker", || {
        registry().get(&second).is_some()
    })
    .await;
    let third = relay(&state, &second, &w.clones[1]);
    wait_for("the second fresh worker", || {
        registry().get(&third).is_some()
    })
    .await;

    assert!(registry().get(&ids[1]).is_none() && registry().get(&second).is_none());
    assert_eq!(
        watch_of(&third).created_at,
        101,
        "the queue place travels with the watch"
    );
    assert!(hosted(&state, &third).await);
    for old in [&ids[1], &second] {
        assert!(!hosted(&state, old).await, "{old} was retired");
        assert_eq!(state.store.session_worktree(old).unwrap(), None);
        assert!(!state.store.session_daemon_live(old).unwrap());
    }
    assert!(state.store.session_daemon_live(&third).unwrap());

    sweeps(&state, &w, 14, 1000).await;

    wait_for(
        "the conflict turn to reach the worker holding the goal",
        || has_conflict_turn(&state, &third),
    )
    .await;
    assert!(!has_conflict_turn(&state, &ids[1]) && !has_conflict_turn(&state, &second));
    assert_eq!(merged_order(&w), vec!["1", "2", "3"]);

    resolve_four_by_hand(&w);
    sweeps(&state, &w, 6, 5000).await;
    assert_eq!(watch_of(&third).state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["1", "2", "3", "4"]);
    let mut all = ids.clone();
    all.extend([second, third]);
    disarm(&all);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_daemon_that_died_between_the_hand_off_and_hosting_restarts_with_one_holder() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::new(&[("1", ""), ("2", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let before = mock_state(w.dir.path());
    let ids = arm_sessions(&before, &w).await;
    // The store half ran; the daemon died before it could host the worker.
    before
        .store
        .set_session_worktree(&ids[0], &w.clones[0].display().to_string())
        .unwrap();
    let successor = before
        .store
        .create_relay_successor(&ids[0], "stalled", "", "BRIEF")
        .unwrap();

    let after = mock_state(w.dir.path());
    crate::serve::resurrect_fleet(after.clone()).await;
    assert!(
        hosted(&after, &successor).await,
        "the fleet flag came with the hand-off"
    );
    assert!(!hosted(&after, &ids[0]).await);

    sweeps(&after, &w, 8, 1000).await;
    assert!(registry().get(&ids[0]).is_none());
    assert_eq!(watch_of(&successor).state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["1", "2"]);
    let mut all = ids;
    all.push(successor);
    disarm(&all);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shipping_a_session_that_handed_its_goal_on_is_refused_and_points_at_the_holder() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::new(&[("1", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;
    state
        .store
        .set_session_worktree(&ids[0], &w.clones[0].display().to_string())
        .unwrap();
    let successor = state
        .store
        .create_relay_successor(&ids[0], "worn", "", "BRIEF")
        .unwrap();
    let request = crate::serve_git::ship::ShipRequest::default();
    let out =
        crate::serve_git::ship::ship_session(&state, &ids[0], &request, &w.dir.path().join("gh"))
            .await
            .ok()
            .unwrap();
    assert!(!out.ok);
    let detail = serde_json::to_string(&out.steps).unwrap();
    assert!(
        detail.contains(&format!("moved to {successor}; ship from there")),
        "{detail}"
    );
    assert_eq!(git(&w.clones[0], &["status", "--porcelain"]), "");
    disarm(&ids);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_head_nobody_hosts_is_hosted_on_demand_and_lands_first() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::new(&[("1", ""), ("2", ""), ("3", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;
    park_head(&state, &w, &ids).await;
    git(&w.clones[0], &["checkout", "--", "."]);
    // A restart that restored only the newest sessions left the head out of the registry.
    state.registry.remove(&ids[0]).await.unwrap().shutdown();

    sweeps(&state, &w, 12, 1000).await;

    assert!(hosted(&state, &ids[0]).await);
    assert_eq!(merged_order(&w), vec!["1", "2", "3"]);
    disarm(&ids);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_archived_head_is_skipped_with_a_reason_and_the_queue_moves_on() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::new(&[("1", ""), ("2", ""), ("3", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;
    park_head(&state, &w, &ids).await;
    git(&w.clones[0], &["checkout", "--", "."]);
    // Archived behind the watch's back (a store write, as an older daemon or another host does).
    state.registry.remove(&ids[0]).await.unwrap().shutdown();
    state.store.archive_session(&ids[0]).unwrap();

    sweeps(&state, &w, 12, 1000).await;

    let head = watch_of(&ids[0]);
    let q = head.queue.as_ref().unwrap();
    assert_eq!(q.state, QueueState::Skipped);
    assert_eq!(
        q.reason.as_deref(),
        Some("its session is not hosted: it was archived")
    );
    assert_eq!(merged_order(&w), vec!["2", "3"]);
    assert!(
        !hosted(&state, &ids[0]).await,
        "an archived session is never resurrected"
    );

    // The owner un-archives it and retries: the PR re-enters, behind nobody.
    state.store.unarchive_session(&ids[0]).unwrap();
    registry()
        .queue_action(Some(&ids[0]), None, Action::Retry)
        .unwrap();
    sweeps(&state, &w, 6, 5000).await;
    assert_eq!(watch_of(&ids[0]).state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["2", "3", "1"]);
    disarm(&ids);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archiving_disarms_the_watch_or_hands_it_to_the_worker_holding_the_goal() {
    use axum::extract::{Path as AxPath, State};
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::new(&[("1", ""), ("2", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;

    crate::serve_lifecycle::archive_session(State(state.clone()), AxPath(ids[0].clone())).await;
    assert!(
        registry().get(&ids[0]).is_none(),
        "an archived session's watch is gone"
    );

    let successor = state
        .store
        .create_relay_successor(&ids[1], "wedged", "", "BRIEF")
        .unwrap();
    crate::serve_lifecycle::archive_session(State(state.clone()), AxPath(ids[1].clone())).await;
    assert!(registry().get(&ids[1]).is_none());
    assert_eq!(watch_of(&successor).created_at, 101);
    let mut all = ids;
    all.push(successor);
    disarm(&all);
}

/// A clone one commit behind `origin/main`, with a merge of it started and left behind.
fn leave_merge_behind(w: &World, marker: bool) -> &Path {
    let root = w.dir.path();
    git(root, &["clone", "-q", "remote.git", "pusher"]);
    let pusher = root.join("pusher");
    git(&pusher, &["config", "user.name", "P"]);
    git(&pusher, &["config", "user.email", "p@example.invalid"]);
    std::fs::write(pusher.join("later.txt"), "later\n").unwrap();
    git(&pusher, &["add", "."]);
    git(&pusher, &["commit", "-q", "-m", "later"]);
    git(&pusher, &["push", "-q", "origin", "main"]);
    let clone = &w.clones[0];
    git(clone, &["fetch", "-q", "origin"]);
    git(clone, &["merge", "--no-commit", "--no-ff", "origin/main"]);
    git(clone, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    if marker {
        let path = git(clone, &["rev-parse", "--git-path", "forge-update-merge"]);
        std::fs::write(clone.join(path), "").unwrap();
    }
    clone
}

#[test]
fn a_merge_a_crashed_update_left_behind_is_rolled_back_and_the_step_retried() {
    let w = World::new(&[("1", "")]);
    let clone = leave_merge_behind(&w, true);
    let head = git(clone, &["rev-parse", "HEAD"]);

    let step = run_step(clone, "main", &[]);
    assert_eq!(
        step,
        Step::Wait("rolled back an interrupted update; retrying".into())
    );
    assert!(!Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(git(clone, &["rev-parse", "HEAD"]), head);

    match run_step(clone, "main", &[]) {
        Step::Pushed { merged, .. } => assert!(merged),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        git(&w.bare(), &["rev-parse", "refs/heads/feat-1"]),
        git(clone, &["rev-parse", "HEAD"])
    );
}

#[test]
fn a_merge_nobody_marked_is_the_sessions_own_and_is_left_alone() {
    let w = World::new(&[("1", "")]);
    let clone = leave_merge_behind(&w, false);
    assert_eq!(
        run_step(clone, "main", &[]),
        Step::Wait("a merge is already in progress in the worktree".into())
    );
    git(clone, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_daemon_rolls_back_a_crashed_update_when_it_starts() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::new(&[("1", ""), ("2", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;
    let clone = leave_merge_behind(&w, true);

    crate::serve_custody::recover_interrupted_updates(&state).await;
    assert!(!Command::new("git")
        .arg("-C")
        .arg(clone)
        .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .output()
        .unwrap()
        .status
        .success());
    assert_eq!(
        watch_of(&ids[0]).last_action.as_deref(),
        Some("rolled back an interrupted update; retrying")
    );
    sweeps(&state, &w, 10, 1000).await;
    assert_eq!(merged_order(&w), vec!["1", "2"]);
    disarm(&ids);
}
