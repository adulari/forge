use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use forge_types::goal_envelope::EnvelopeState;

use super::*;
use crate::serve::tests::FORGE_DB_LOCK;
use crate::serve::{start_session, CreateSessionReq, SessionRegistry};

const NOW: i64 = 1_800_000_000;

fn window(pct: f64, resets_at: i64, now: i64) -> forge_store::SubscriptionWindow {
    forge_store::SubscriptionWindow {
        provider: "claude-cli".into(),
        window_kind: "weekly".into(),
        status: "ok".into(),
        resets_at: Some(resets_at),
        fraction: Some(pct / 100.0),
        updated_at: now,
    }
}

fn sources(pct: f64, resets_at: i64, now: i64) -> EnvelopeSources {
    EnvelopeSources {
        windows: vec![window(pct, resets_at, now)],
        session_usd: 0.0,
        now,
    }
}

fn drain(numbers: &[u64], parallel: usize, envelope: Option<EnvelopeState>) -> Drain {
    Drain::new(
        "d1".into(),
        "/repo".into(),
        numbers,
        parallel,
        envelope.as_ref().map(|_| "claude:30%w".to_string()),
        envelope,
        (true, Mode::Fix, "squash".into()),
        NOW,
    )
}

fn busy() -> LiveSession {
    LiveSession {
        busy: true,
        turns_finished: 0,
        ..LiveSession::default()
    }
}

fn idle() -> LiveSession {
    LiveSession {
        turns_finished: 1,
        ..LiveSession::default()
    }
}

fn step(
    d: &mut Drain,
    live: &[(&str, LiveSession)],
    watches: &[(&str, WatchView)],
    sources: &EnvelopeSources,
) -> Vec<u64> {
    let live = live.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let watches = watches
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    advance(
        d,
        &Inputs {
            live: &live,
            watches: &watches,
            sources,
            spent_usd: 0.0,
            now: sources.now,
        },
    )
}

fn start(d: &mut Drain, number: u64, session: &str) {
    d.record_start(number, Ok((session.into(), format!("#{number}"))), NOW);
}

fn pr(number: u64) -> WatchView {
    WatchView {
        pr_number: Some(number),
        pr_url: Some(format!("https://github.com/acme/forge/pull/{number}")),
        ..WatchView::default()
    }
}

fn status_of(d: &Drain, number: u64) -> ItemStatus {
    d.items.iter().find(|i| i.number == number).unwrap().status
}

#[test]
fn runs_at_most_parallel_items_and_starts_the_next_when_one_finishes() {
    let src = sources(10.0, NOW + 100_000, NOW);
    let mut d = drain(&[12, 15, 19, 20], 2, None);
    assert_eq!(step(&mut d, &[], &[], &src), vec![12, 15]);
    start(&mut d, 12, "s12");
    start(&mut d, 15, "s15");

    let both = [("s12", busy()), ("s15", busy())];
    assert!(
        step(&mut d, &both, &[], &src).is_empty(),
        "both slots are taken"
    );

    // 12 opens a PR: its slot frees, the PR is watched, 19 starts.
    let live = [("s12", idle()), ("s15", busy())];
    let watches = [("s12", pr(7))];
    assert_eq!(step(&mut d, &live, &watches, &src), vec![19]);
    assert_eq!(status_of(&d, 12), ItemStatus::PrOpen);
    assert_eq!(
        d.items[0].pr_url.as_deref(),
        Some("https://github.com/acme/forge/pull/7")
    );
    start(&mut d, 19, "s19");

    // 15 asks the owner a question: it frees its slot too, without counting as done.
    let live = [
        ("s12", idle()),
        (
            "s15",
            LiveSession {
                waiting: true,
                busy: true,
                ..busy()
            },
        ),
        ("s19", busy()),
    ];
    assert_eq!(step(&mut d, &live, &watches, &src), vec![20]);
    assert_eq!(status_of(&d, 15), ItemStatus::Waiting);
    start(&mut d, 20, "s20");

    // Everything has been started; the drain is finished once nothing is running.
    let live = [
        ("s12", idle()),
        ("s15", idle()),
        ("s19", idle()),
        ("s20", idle()),
    ];
    let mut merged = pr(7);
    merged.merged = true;
    let watches = [("s12", merged), ("s19", pr(9))];
    assert!(step(&mut d, &live, &watches, &src).is_empty());
    assert_eq!(d.status, DrainStatus::Finished);
    assert_eq!(status_of(&d, 12), ItemStatus::Merged);
    assert_eq!(status_of(&d, 19), ItemStatus::PrOpen);
    assert_eq!(status_of(&d, 15), ItemStatus::Done);
    assert_eq!(status_of(&d, 20), ItemStatus::Done);
}

#[test]
fn a_failed_session_or_a_blocked_watch_frees_the_slot_and_counts_as_blocked() {
    let src = sources(10.0, NOW + 100_000, NOW);
    let mut d = drain(&[1, 2, 3], 2, None);
    step(&mut d, &[], &[], &src);
    start(&mut d, 1, "s1");
    start(&mut d, 2, "s2");
    let failed = LiveSession {
        failed: true,
        ..idle()
    };
    let blocked = WatchView {
        blocked: Some("checks keep failing".into()),
        ..pr(4)
    };
    let starts = step(
        &mut d,
        &[("s1", failed), ("s2", idle())],
        &[("s2", blocked)],
        &src,
    );
    assert_eq!(starts, vec![3]);
    assert_eq!(status_of(&d, 1), ItemStatus::Failed);
    assert_eq!(status_of(&d, 2), ItemStatus::Blocked);
    assert_eq!(d.items[1].outcome.as_deref(), Some("checks keep failing"));
    assert_eq!(d.counts().blocked, 2);
}

#[test]
fn a_session_that_vanished_is_blocked_and_a_fresh_one_is_given_time_to_start() {
    let src = sources(10.0, NOW + 100_000, NOW);
    let mut d = drain(&[1, 2], 2, None);
    step(&mut d, &[], &[], &src);
    start(&mut d, 1, "s1");
    start(&mut d, 2, "s2");
    // s2 has not picked up its prompt yet (no turn finished, not busy): still running.
    step(&mut d, &[("s2", LiveSession::default())], &[], &src);
    assert_eq!(status_of(&d, 1), ItemStatus::Blocked);
    assert_eq!(status_of(&d, 2), ItemStatus::Running);
    // Long after, with the turn counter still 0 (a restart reset it), it is idle work.
    let late = sources(10.0, NOW + 100_000, NOW + START_GRACE_SECS + 5);
    step(&mut d, &[("s2", LiveSession::default())], &[], &late);
    assert_eq!(status_of(&d, 2), ItemStatus::Done);
}

#[test]
fn a_spent_envelope_stops_new_starts_and_the_window_reset_resumes_them() {
    let reset = NOW + 3600;
    let spec = EnvelopeSpec::parse("claude:30%w", None).unwrap();
    let env = EnvelopeState::new(spec, 50.0, Some(reset));
    let mut d = drain(&[1, 2, 3, 4], 2, Some(env));

    let src = sources(50.0, reset, NOW);
    assert_eq!(step(&mut d, &[], &[], &src), vec![1, 2]);
    start(&mut d, 1, "s1");
    start(&mut d, 2, "s2");

    let src = sources(60.0, reset, NOW + 60);
    let both = [("s1", busy()), ("s2", busy())];
    assert!(step(&mut d, &both, &[], &src).is_empty());
    assert_eq!(d.status, DrainStatus::Active);

    // Item 1 lands its PR while the window crosses the 30-point slice.
    let src = sources(81.0, reset, NOW + 120);
    let live = [("s1", idle()), ("s2", busy())];
    assert!(step(&mut d, &live, &[("s1", pr(5))], &src).is_empty());
    assert_eq!(d.status, DrainStatus::Paused);
    assert!(d
        .paused_reason
        .as_deref()
        .unwrap()
        .contains("Envelope spent"));
    assert!(d.envelope.as_ref().unwrap().spent);

    // Still spent a bit later: still nothing, even with a slot free.
    let src = sources(83.0, reset, NOW + 1800);
    assert!(step(&mut d, &live, &[("s1", pr(5))], &src).is_empty());
    assert_eq!(d.status, DrainStatus::Paused);

    // The window resets: the drain rebaselines on the new reading and starts item 3.
    let src = sources(2.0, reset + 7 * 86_400, reset + 30);
    assert_eq!(step(&mut d, &live, &[("s1", pr(5))], &src), vec![3]);
    assert_eq!(d.status, DrainStatus::Active);
    assert!(d.paused_reason.is_none());
    let env = d.envelope.as_ref().unwrap();
    assert!(!env.spent);
    assert!(env.used < 1.0);
}

#[test]
fn a_stale_quota_reading_holds_the_queue_without_pausing() {
    let reset = NOW + 3600;
    let spec = EnvelopeSpec::parse("claude:30%w", None).unwrap();
    let mut d = drain(
        &[1, 2],
        2,
        Some(EnvelopeState::new(spec, 50.0, Some(reset))),
    );
    let stale = sources(50.0, reset, NOW - 3600);
    let at_now = EnvelopeSources { now: NOW, ..stale };
    assert!(step(&mut d, &[], &[], &at_now).is_empty());
    assert_eq!(d.status, DrainStatus::Active);
    assert!(d.envelope.as_ref().unwrap().unverified);
}

#[test]
fn a_dollar_envelope_counts_what_the_drains_sessions_cost() {
    let spec = EnvelopeSpec::parse("$5", None).unwrap();
    let mut d = drain(&[1, 2], 1, Some(EnvelopeState::new(spec, 0.0, None)));
    let src = sources(0.0, NOW + 1, NOW);
    let live = std::collections::HashMap::new();
    let watches = std::collections::HashMap::new();
    let mut inputs = Inputs {
        live: &live,
        watches: &watches,
        sources: &src,
        spent_usd: 1.0,
        now: NOW,
    };
    assert_eq!(advance(&mut d, &inputs), vec![1]);
    inputs.spent_usd = 5.5;
    assert!(advance(&mut d, &inputs).is_empty());
    assert_eq!(d.status, DrainStatus::Paused);
    assert!(d
        .paused_reason
        .as_deref()
        .unwrap()
        .contains("larger budget"));
}

#[test]
fn cancel_drops_the_queue_and_leaves_running_items_alone() {
    let src = sources(10.0, NOW + 100_000, NOW);
    let mut d = drain(&[1, 2, 3], 1, None);
    step(&mut d, &[], &[], &src);
    start(&mut d, 1, "s1");
    d.cancel(NOW + 5);
    assert_eq!(d.status, DrainStatus::Cancelled);
    assert_eq!(status_of(&d, 1), ItemStatus::Running);
    assert_eq!(status_of(&d, 2), ItemStatus::Cancelled);
    assert!(step(&mut d, &[("s1", idle())], &[("s1", pr(3))], &src).is_empty());
    assert_eq!(status_of(&d, 1), ItemStatus::PrOpen);
}

#[test]
fn the_brief_line_names_started_merged_waiting_and_blocked() {
    let mut d = drain(&[1, 2, 3, 4, 5, 6, 7, 8], 8, None);
    let statuses = [
        ItemStatus::Merged,
        ItemStatus::Merged,
        ItemStatus::Waiting,
        ItemStatus::PrOpen,
        ItemStatus::Blocked,
        ItemStatus::Running,
        ItemStatus::Running,
        ItemStatus::Queued,
    ];
    for (i, (item, status)) in d.items.iter_mut().zip(statuses).enumerate() {
        item.status = status;
        item.session_id = (status != ItemStatus::Queued).then(|| format!("s{i}"));
    }
    assert_eq!(
        d.line(),
        "Drain: 7 started, 2 merged, 2 waiting on you, 1 blocked, 1 queued"
    );
    d.mode = Mode::Merge;
    assert_eq!(
        d.counts().waiting,
        1,
        "a merge-mode PR waits on the watch, not the owner"
    );
    d.status = DrainStatus::Paused;
    assert!(d.line().ends_with("(paused)"));
}

#[test]
fn drains_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drains.json");
    let reg = DrainRegistry::new(Some(path.clone()));
    let reset = NOW + 3600;
    let spec = EnvelopeSpec::parse("claude:30%w", None).unwrap();
    let mut d = drain(
        &[1, 2, 3],
        1,
        Some(EnvelopeState::new(spec, 50.0, Some(reset))),
    );
    start(&mut d, 1, "s1");
    reg.insert(d.clone());
    reg.with("d1", |d| d.paused_reason = Some("x".into()))
        .unwrap();

    let restored = DrainRegistry::new(Some(path));
    let got = restored.get("d1").unwrap();
    assert_eq!(got.items, d.items);
    assert_eq!(got.envelope, d.envelope);
    assert_eq!(got.paused_reason.as_deref(), Some("x"));

    // The restored drain carries on: item 1 finished while the daemon was down.
    let src = sources(52.0, reset, NOW);
    let mut got = got;
    let starts = step(&mut got, &[("s1", idle())], &[("s1", pr(2))], &src);
    assert_eq!(starts, vec![2]);
}

fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// A `gh` that knows four issues (numbers 1-4).
fn fake_gh(dir: &Path) -> Gh {
    let path = dir.join("gh");
    let script = r#"#!/bin/sh
if [ "$2" = view ]; then
  n="$3"
  printf '{"number":%s,"title":"Issue %s","body":"fix it","url":"https://github.com/acme/forge/issues/%s","state":"OPEN","labels":[],"author":{"login":"floris"},"comments":[]}' "$n" "$n" "$n"
else
  echo '[]'
fi
"#;
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    Gh::with_program(path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drain_over_mock_sessions_starts_two_then_the_rest_as_they_finish() {
    let _env = FORGE_DB_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FORGE_DB", dir.path().join("drain.db"));
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let store = Arc::new(crate::open_store().unwrap());
    let state = Arc::new(DaemonState {
        registry: Arc::new(SessionRegistry::new()),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store,
        base: "/tok".into(),
        mock: true,
        default_cwd: repo.display().to_string(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    });
    let seed = start_session(
        &state,
        CreateSessionReq {
            cwd: Some(repo.display().to_string()),
            ..Default::default()
        },
    )
    .await
    .ok()
    .unwrap();
    let gh_dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(gh_dir.path());
    let reg = DrainRegistry::new(Some(dir.path().join("drains.json")));

    let created = create(
        &state,
        &reg,
        CreateRequest {
            session: Some(seed.session_id.clone()),
            cwd: None,
            numbers: vec![1, 2, 3, 4, 3],
            parallel: Some(2),
            budget: None,
            worktree: true,
            mode: Mode::Fix,
            method: None,
        },
    )
    .await
    .ok()
    .unwrap();
    assert_eq!(created.items.len(), 4, "duplicates collapse");

    let src = EnvelopeSources::from_store(&state.store, 0.0);
    sweep(&state, &gh, &reg, &src).await;
    let first = reg.get(&created.id).unwrap();
    let started: Vec<_> = first
        .items
        .iter()
        .filter(|i| i.session_id.is_some())
        .collect();
    assert_eq!(started.len(), 2, "two at a time");
    assert_eq!(first.items[2].status, ItemStatus::Queued);
    assert!(first.line().starts_with("Drain: 2 started"));
    for item in &started {
        let id = item.session_id.as_deref().unwrap();
        assert!(crate::serve_pr_watch::is_watching(id));
        let handle = state.registry.get(id).await.unwrap();
        let mode = handle
            .snapshot_rx
            .borrow()
            .snapshot
            .permission_mode
            .to_lowercase();
        assert!(!mode.contains("auto"), "drain session escalated to {mode}");
    }

    // The mock turns end on their own; each pass then starts what fits until all four ran.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let src = EnvelopeSources::from_store(&state.store, 0.0);
        sweep(&state, &gh, &reg, &src).await;
        let now = reg.get(&created.id).unwrap();
        let running = now
            .items
            .iter()
            .filter(|i| i.status == ItemStatus::Running)
            .count();
        assert!(running <= 2, "never more than parallel running");
        if now.items.iter().all(|i| i.session_id.is_some()) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "drain stalled: {}",
            now.line()
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // A budget that does not parse is refused before anything is queued.
    let bad_budget = create(
        &state,
        &reg,
        CreateRequest {
            session: None,
            cwd: Some(repo.display().to_string()),
            numbers: vec![4],
            parallel: None,
            budget: Some("not-a-budget".into()),
            worktree: true,
            mode: Mode::Fix,
            method: None,
        },
    )
    .await;
    assert!(bad_budget.is_err(), "a bad budget is refused");

    let brief = brief_line(&reg, 0).unwrap();
    assert_eq!(brief.id, created.id);
    assert!(brief.line.starts_with("Drain: 4 started"));

    std::env::remove_var("FORGE_DB");
}
