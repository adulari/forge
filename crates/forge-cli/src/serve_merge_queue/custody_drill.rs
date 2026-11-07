//! The custody drill (scripts in `~/.cache/forge-burn-demo/run4/custody/`). It prints what a
//! night of relays does to a merge queue and uses only APIs that exist on both sides of the fix,
//! so the same file runs against the branch before it. With `FORGE_DRILL_EXPECT_FIXED=1` it also
//! asserts the fixed behaviour; without it, it only reports.

use super::tests::{
    arm_sessions, disarm, merged_order, mock_state, resolve_four_by_hand, sweeps, watch_of, World,
};
use crate::serve::tests::FORGE_DB_LOCK;
use crate::serve_pr_watch::registry;

fn conflict_turn(state: &crate::serve::DaemonState, id: &str) -> bool {
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

/// Exactly what the goal driver does on a relay (`launch_successor`), minus the model.
async fn relay(
    state: &crate::serve::DaemonState,
    from: &str,
    worktree: &std::path::Path,
) -> String {
    state
        .store
        .set_session_worktree(from, &worktree.display().to_string())
        .unwrap();
    let to = state
        .store
        .create_relay_successor(from, "wedged", "same failing set", "BRIEF")
        .unwrap();
    state.store.set_session_daemon_live(&to, true).unwrap();
    assert!(state.registry.launch_relay(to.clone()));
    for _ in 0..200 {
        if state.registry.get(&to).await.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    to
}

fn short(id: &str) -> &str {
    &id[..id.len().min(6)]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drill() {
    let _env = FORGE_DB_LOCK.lock().await;
    let fixed = std::env::var("FORGE_DRILL_EXPECT_FIXED").is_ok();

    println!("== 1. PR 4's goal moves to a fresh worker, then PR 4 hits a base conflict");
    let w = World::new(&[("1", "from one"), ("4", "from four"), ("2", ""), ("3", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    crate::serve_relay::start(&state);
    let ids = arm_sessions(&state, &w).await;
    sweeps(&state, &w, 1, 900).await;
    let second = relay(&state, &ids[1], &w.clones[1]).await;
    let third = relay(&state, &second, &w.clones[1]).await;
    println!(
        "custody chain: {} -> {} -> {}",
        short(&ids[1]),
        short(&second),
        short(&third)
    );
    let holder = [&ids[1], &second, &third]
        .into_iter()
        .find(|id| registry().get(id).is_some())
        .unwrap();
    println!("PR #4 watch is held by {}", short(holder));
    for (name, id) in [
        ("worker 1", &ids[1]),
        ("worker 2", &second),
        ("worker 3", &third),
    ] {
        println!(
            "  {name} {}: hosted={} worktree={:?} fleet_slot={}",
            short(id),
            state.registry.get(id).await.is_some(),
            state.store.session_worktree(id).unwrap().map(|_| "set"),
            state.store.session_daemon_live(id).unwrap()
        );
    }
    sweeps(&state, &w, 14, 1000).await;
    for _ in 0..200 {
        if [&ids[1], &second, &third]
            .into_iter()
            .any(|id| conflict_turn(&state, id))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    for (name, id) in [
        ("worker 1", &ids[1]),
        ("worker 2", &second),
        ("worker 3", &third),
    ] {
        println!(
            "  conflict fix turn reached {name} {}: {}",
            short(id),
            conflict_turn(&state, id)
        );
    }
    resolve_four_by_hand(&w);
    sweeps(&state, &w, 6, 5000).await;
    println!("merged in order: {:?}", merged_order(&w));
    if fixed {
        assert!(conflict_turn(&state, &third) && !conflict_turn(&state, &ids[1]));
        assert_eq!(merged_order(&w), vec!["1", "2", "3", "4"]);
    }
    let mut all = ids.clone();
    all.extend([second, third]);
    disarm(&all);

    println!("== 2. the head's worker was replaced and the superseded worker is archived");
    let w = World::new(&[("1", ""), ("2", ""), ("3", "")]);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    let state = mock_state(w.dir.path());
    crate::serve_relay::start(&state);
    let ids = arm_sessions(&state, &w).await;
    // PR 1 is the head but cannot land yet: its worktree has an uncommitted edit.
    std::fs::write(w.clones[0].join("shared.txt"), "edited\n").unwrap();
    sweeps(&state, &w, 2, 900).await;
    let next = relay(&state, &ids[0], &w.clones[0]).await;
    std::process::Command::new("git")
        .arg("-C")
        .arg(&w.clones[0])
        .args(["checkout", "--", "."])
        .status()
        .unwrap();
    crate::serve_lifecycle::archive_session(
        axum::extract::State(state.clone()),
        axum::extract::Path(ids[0].clone()),
    )
    .await;
    sweeps(&state, &w, 14, 1000).await;
    println!("merged in order: {:?}", merged_order(&w));
    for id in ids.iter().chain([&next]) {
        if let Some(watch) = registry().get(id) {
            println!(
                "  watch {} PR #{:?} {:?}: {}",
                short(id),
                watch.pr_number,
                watch.state,
                watch.hold.as_deref().unwrap_or("-")
            );
        }
    }
    let _ = watch_of;
    if fixed {
        assert_eq!(merged_order(&w), vec!["1", "2", "3"]);
    }
    let mut all = ids;
    all.push(next);
    disarm(&all);
}
