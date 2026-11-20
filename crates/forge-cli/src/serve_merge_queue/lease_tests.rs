use std::sync::Arc;

use super::*;
use crate::serve_merge_queue::tests::{git, merged_order, World};
use crate::serve_merge_queue::{apply, settle_leases, snapshot, Action, Queue, QueueState};
use crate::serve_pr_watch::{advance, Mode, Phase, Watch};

const REPO: &str = "o/r";

fn key(w: &World) -> String {
    format!("{} @ main", w.bare().display())
}

async fn acquire(l: &Leases, w: &World, pr: u64, now: i64) -> Lease {
    l.acquire(&w.gh(), &w.clones[0], &key(w), "main", Some(pr), now)
        .await
}

fn api_calls(w: &World) -> usize {
    w.gh_log().lines().filter(|l| l.starts_with("api ")).count()
}

#[tokio::test]
async fn one_daemon_holds_renews_and_gives_the_lease_back() {
    let w = World::new(&[("1", "")]);
    let a = Leases::for_test("laptop", REPO);
    let b = Leases::for_test("ssu-dev", REPO);

    let Lease::Held(view) = acquire(&a, &w, 1, 1000).await else {
        panic!("a free lease is taken");
    };
    assert_eq!(
        (view.status, view.holder.as_deref(), view.pr),
        (Status::Mine, Some("laptop"), Some(1))
    );
    assert_eq!(view.expires_at, Some(1000 + STALE_SECS));

    let Lease::Busy(view) = acquire(&b, &w, 9, 1010).await else {
        panic!("a live lease is not shared");
    };
    assert_eq!(
        (view.status, view.holder.as_deref(), view.pr),
        (Status::Other, Some("laptop"), Some(1))
    );
    assert_eq!(view.landing(), "landing on laptop: PR #1");

    // A renewal inside the heartbeat window costs no GitHub call.
    let calls = api_calls(&w);
    assert!(matches!(acquire(&a, &w, 1, 1030).await, Lease::Held(_)));
    assert_eq!(api_calls(&w), calls);
    // After it, the holder renews and the other still waits.
    assert!(matches!(acquire(&a, &w, 1, 1070).await, Lease::Held(_)));
    assert!(matches!(acquire(&b, &w, 9, 1075).await, Lease::Busy(_)));

    a.release(&w.gh(), &key(&w), 1080).await;
    assert!(!a.holds(&key(&w)));
    let Lease::Held(view) = acquire(&b, &w, 9, 1081).await else {
        panic!("a released lease is free at once");
    };
    assert_eq!(view.holder.as_deref(), Some("ssu-dev"));
    assert_eq!(view.note, None, "nothing was stolen");
}

#[tokio::test]
async fn a_lease_nobody_renews_goes_stale_and_is_taken_over() {
    let w = World::new(&[("1", "")]);
    let a = Leases::for_test("laptop", REPO);
    let b = Leases::for_test("ssu-dev", REPO);
    assert!(matches!(acquire(&a, &w, 1, 0).await, Lease::Held(_)));

    assert!(
        matches!(acquire(&b, &w, 2, STALE_SECS).await, Lease::Busy(_)),
        "exactly at the limit it is still live"
    );
    let Lease::Held(view) = acquire(&b, &w, 2, STALE_SECS + 1).await else {
        panic!("a stale lease is taken");
    };
    assert_eq!(
        view.note.as_deref(),
        Some("took over a stale lease from laptop")
    );

    // The old holder wakes up: its cache says it holds the lease, GitHub says it does not.
    let Lease::Busy(view) = acquire(&a, &w, 1, STALE_SECS + 120).await else {
        panic!("the old holder must give way");
    };
    assert_eq!(view.holder.as_deref(), Some("ssu-dev"));
    assert!(!a.holds(&key(&w)));
}

#[tokio::test]
async fn a_restarted_daemon_renews_its_own_lease_instead_of_waiting_it_out() {
    let w = World::new(&[("1", "")]);
    assert!(matches!(
        acquire(&Leases::for_test("laptop", REPO), &w, 1, 0).await,
        Lease::Held(_)
    ));
    let again = Leases::for_test("laptop", REPO);
    assert!(matches!(acquire(&again, &w, 1, 30).await, Lease::Held(_)));
}

#[tokio::test]
async fn losing_the_race_to_move_the_ref_is_contention_not_a_double_hold() {
    let w = World::new(&[("1", "")]);
    let a = Leases::for_test("laptop", REPO);
    let b = Leases::for_test("ssu-dev", REPO);
    let refname = "refs/forge/merge-queue/main";
    let root = &w.clones[0];
    assert!(matches!(acquire(&a, &w, 1, 0).await, Lease::Held(_)));
    let (sha, _) = a.read(&w.gh(), root, REPO, refname).await.unwrap().unwrap();
    let body = |owner: &str, now| Body {
        owner: owner.into(),
        host: owner.into(),
        pr: None,
        renewed_at: now,
        released: false,
    };
    // Both read `sha` and both want to take it over; the second ref move is not a fast-forward.
    let first = b
        .write(
            &w.gh(),
            root,
            REPO,
            refname,
            "main",
            &body("b", 1),
            Some(&sha),
        )
        .await;
    let second = a
        .write(
            &w.gh(),
            root,
            REPO,
            refname,
            "main",
            &body("a", 2),
            Some(&sha),
        )
        .await;
    assert!(first.is_ok());
    assert!(matches!(second, Err(Fail::Contended)), "{second:?}");
    // Creating a ref that exists is the same contention.
    let create = a
        .write(&w.gh(), root, REPO, refname, "main", &body("a", 3), None)
        .await;
    assert!(matches!(create, Err(Fail::Contended)), "{create:?}");
}

#[tokio::test]
async fn when_github_refuses_the_lease_the_daemon_lands_on_its_own_and_asks_again_later() {
    let w = World::new(&[("1", "")]);
    let a = Leases::for_test("laptop", REPO);
    std::fs::write(w.dir.path().join("api-down"), "").unwrap();
    let Lease::Degraded(view) = acquire(&a, &w, 1, 0).await else {
        panic!("a refused lease degrades");
    };
    assert_eq!(view.status, Status::Unavailable);
    assert!(
        view.landing().contains("shared queue unavailable"),
        "{}",
        view.landing()
    );
    assert!(view.landing().contains("landing on its own"));

    let calls = api_calls(&w);
    assert!(matches!(acquire(&a, &w, 1, 60).await, Lease::Degraded(_)));
    assert_eq!(api_calls(&w), calls, "no GitHub call while backing off");

    std::fs::remove_file(w.dir.path().join("api-down")).unwrap();
    assert!(matches!(
        acquire(&a, &w, 1, DEGRADED_SECS + 1).await,
        Lease::Held(_)
    ));
}

#[tokio::test]
async fn a_remote_that_is_not_on_github_needs_no_lease() {
    let w = World::new(&[("1", "")]);
    let a = Leases::new(Identity::new("laptop", Path::new("/x")));
    // The test remote is a local path: nothing to coordinate on, and no GitHub call is made.
    assert_eq!(acquire(&a, &w, 1, 0).await, Lease::Off);
    assert_eq!(api_calls(&w), 0);
}

/// One simulated daemon: its own leases and the watches it hosts (by clone index).
struct Daemon {
    queue: Queue,
    leases: Arc<Leases>,
    ws: Vec<(usize, Watch)>,
}

impl Daemon {
    fn new(host: &str, clones: &[usize], created: i64) -> Self {
        let leases = Arc::new(Leases::for_test(host, REPO));
        Self {
            queue: Queue::Shared(leases.clone()),
            leases,
            ws: clones
                .iter()
                .map(|i| {
                    let id = format!("s{i}");
                    (
                        *i,
                        Watch::new(&id, Mode::Merge, "squash", created + *i as i64),
                    )
                })
                .collect(),
        }
    }

    fn watch(&self, clone: usize) -> &Watch {
        &self.ws.iter().find(|(i, _)| *i == clone).unwrap().1
    }

    fn all(&self) -> Vec<Watch> {
        self.ws.iter().map(|(_, w)| w.clone()).collect()
    }

    /// The daemon's sweep: every active watch has its turn, then idle leases go back.
    async fn tick(&mut self, w: &World, now: i64) {
        let gh = w.gh();
        for n in 0..self.ws.len() {
            if !self.ws[n].1.active() {
                continue;
            }
            let mut next = self.ws[n].1.clone();
            let all = self.all();
            let verify = Some(w.verify());
            advance(
                &self.queue,
                &gh,
                &w.clones[self.ws[n].0],
                &mut next,
                all,
                true,
                now,
                verify,
            )
            .await;
            self.ws[n].1 = next;
        }
        settle_leases(&self.queue, &gh, &self.all(), now).await;
    }

    fn holds_lease(&self, w: &World) -> bool {
        self.leases.holds(&key(w))
    }

    fn landing(&self) -> Vec<usize> {
        self.ws
            .iter()
            .filter(|(_, w)| {
                w.active()
                    && w.queue
                        .as_ref()
                        .is_some_and(|q| matches!(q.state, QueueState::Head | QueueState::Checks))
            })
            .map(|(i, _)| *i)
            .collect()
    }
}

/// Two daemons that watch PRs of one repository: at no tick do both update or land, and every PR
/// lands.
#[tokio::test]
async fn two_daemons_on_one_repository_take_turns_landing() {
    let w = World::new(&[("1", ""), ("2", ""), ("3", ""), ("4", "")]);
    let mut laptop = Daemon::new("laptop", &[0, 1], 0);
    let mut ssu = Daemon::new("ssu-dev", &[2, 3], 0);

    for round in 0..40 {
        let now = 1000 + round * 60;
        laptop.tick(&w, now).await;
        ssu.tick(&w, now).await;
        let (a, b) = (laptop.landing(), ssu.landing());
        assert!(
            a.is_empty() || b.is_empty(),
            "round {round}: both daemons are landing: laptop {a:?}, ssu-dev {b:?}"
        );
        if laptop
            .ws
            .iter()
            .chain(ssu.ws.iter())
            .all(|(_, w)| w.state == Phase::Merged)
        {
            break;
        }
    }
    for d in [&laptop, &ssu] {
        for (i, wch) in &d.ws {
            assert_eq!(wch.state, Phase::Merged, "clone {i}: {:?}", wch.queue);
        }
    }
    assert_eq!(merged_order(&w).len(), 4);
    assert!(
        !laptop.holds_lease(&w) && !ssu.holds_lease(&w),
        "idle daemons give the lease back"
    );
    // Every update after the first PR saw the previous PRs' code: the merge order is the verify order.
    let verified = std::fs::read_to_string(w.dir.path().join("verify.log")).unwrap_or_default();
    assert!(verified.lines().count() >= 3, "{verified}");
}

#[tokio::test]
async fn the_waiting_daemon_says_who_is_landing_and_every_surface_row_carries_it() {
    let w = World::new(&[("1", ""), ("2", "")]);
    // The head cannot move (dirty worktree), so the lease stays held and the state is steady.
    std::fs::write(w.clones[0].join("code-1.txt"), "edited, uncommitted\n").unwrap();
    let mut laptop = Daemon::new("laptop", &[0], 0);
    let mut ssu = Daemon::new("ssu-dev", &[1], 1);
    for round in 0..3 {
        laptop.tick(&w, 100 + round * 30).await;
        ssu.tick(&w, 100 + round * 30).await;
    }
    assert!(laptop.holds_lease(&w) && !ssu.holds_lease(&w));

    let waiting = ssu.watch(1);
    let q = waiting.queue.as_ref().unwrap();
    assert_eq!(q.state, QueueState::Queued);
    let lease = q.lease.as_ref().unwrap();
    assert_eq!(
        (lease.status, lease.holder.as_deref(), lease.pr),
        (Status::Other, Some("laptop"), Some(1))
    );
    assert!(
        q.reason
            .as_deref()
            .unwrap()
            .starts_with("landing on laptop: PR #1"),
        "{:?}",
        q.reason
    );
    assert!(waiting.hold.is_some(), "PR Watch holds the merge");
    assert_eq!(
        waiting.state,
        Phase::Watching,
        "the PR keeps its watch and its fix turns"
    );

    let snap = snapshot(&ssu.all());
    assert_eq!(
        snap[0].lease.as_ref().unwrap().text,
        "landing on laptop: PR #1"
    );
    let own = snapshot(&laptop.all());
    assert_eq!(own[0].lease.as_ref().unwrap().text, "landing here: PR #1");
    let json = serde_json::to_value(&snap).unwrap();
    assert_eq!(json[0]["lease"]["status"], "other");
    assert_eq!(json[0]["lease"]["holder"], "laptop");
}

#[tokio::test]
async fn a_dead_daemon_never_blocks_the_queue_for_longer_than_the_stale_limit() {
    let w = World::new(&[("1", ""), ("2", "")]);
    std::fs::write(w.clones[0].join("code-1.txt"), "edited, uncommitted\n").unwrap();
    let mut laptop = Daemon::new("laptop", &[0], 0);
    let mut ssu = Daemon::new("ssu-dev", &[1], 1);
    for now in [990, 1000] {
        laptop.tick(&w, now).await;
        ssu.tick(&w, now).await;
    }
    assert!(matches!(
        ssu.watch(1)
            .queue
            .as_ref()
            .unwrap()
            .lease
            .as_ref()
            .unwrap()
            .status,
        Status::Other
    ));

    // The laptop goes away; ssu-dev keeps polling.
    for step in 1..=25 {
        ssu.tick(&w, 1000 + step * 30).await;
    }
    assert_eq!(
        ssu.watch(1).state,
        Phase::Merged,
        "{:?}",
        ssu.watch(1).queue
    );
    assert_eq!(merged_order(&w), vec!["2"]);
}

#[tokio::test]
async fn hold_and_front_decide_who_wants_the_lease() {
    let w = World::new(&[("1", ""), ("2", ""), ("3", "")]);
    for clone in [0, 1] {
        std::fs::write(
            w.clones[clone].join(format!("code-{}.txt", clone + 1)),
            "edited, uncommitted\n",
        )
        .unwrap();
    }
    let mut laptop = Daemon::new("laptop", &[0, 1], 0);
    let mut ssu = Daemon::new("ssu-dev", &[2], 5);
    for now in [70, 100] {
        laptop.tick(&w, now).await;
        ssu.tick(&w, now).await;
    }
    assert!(laptop.holds_lease(&w), "PR 1 is the laptop's head");
    assert_eq!(
        ssu.watch(2).queue.as_ref().unwrap().state,
        QueueState::Queued
    );

    // Holding the head: PR 2 becomes the laptop's head and keeps the lease; ssu-dev still waits.
    let act = |d: &mut Daemon, session: &str, action| {
        let mut map: std::collections::HashMap<String, Watch> =
            d.ws.iter()
                .map(|(_, w)| (w.session_id.clone(), w.clone()))
                .collect();
        apply(&mut map, session, action).unwrap();
        for (_, w) in &mut d.ws {
            *w = map[&w.session_id].clone();
        }
    };
    act(&mut laptop, "s0", Action::Hold);
    laptop.tick(&w, 130).await;
    ssu.tick(&w, 130).await;
    assert!(laptop.holds_lease(&w));
    assert_eq!(
        laptop
            .watch(1)
            .queue
            .as_ref()
            .unwrap()
            .lease
            .as_ref()
            .unwrap()
            .pr,
        Some(2)
    );
    assert_eq!(
        ssu.watch(2)
            .queue
            .as_ref()
            .unwrap()
            .lease
            .as_ref()
            .unwrap()
            .pr,
        Some(2)
    );

    // The owner holds the laptop's last waiting PR too: the laptop has nothing to land, so the
    // lease goes back and ssu-dev lands PR 3 straight away.
    act(&mut laptop, "s1", Action::Hold);
    laptop.tick(&w, 160).await;
    assert!(!laptop.holds_lease(&w));
    ssu.tick(&w, 160).await;
    assert!(ssu.holds_lease(&w) || ssu.watch(2).state == Phase::Merged);
    for step in 0..6 {
        ssu.tick(&w, 200 + step * 60).await;
    }
    assert_eq!(ssu.watch(2).state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["3"]);

    // Release: the laptop's PRs take the lease again; front puts PR 2 ahead of PR 1.
    for clone in [0, 1] {
        std::fs::write(
            w.clones[clone].join(format!("code-{}.txt", clone + 1)),
            format!("code {}\n", clone + 1),
        )
        .unwrap();
    }
    act(&mut laptop, "s0", Action::Release);
    act(&mut laptop, "s1", Action::Release);
    act(&mut laptop, "s1", Action::Front);
    for step in 0..12 {
        laptop.tick(&w, 1000 + step * 60).await;
    }
    assert_eq!(merged_order(&w), vec!["3", "2", "1"]);
}

#[tokio::test]
async fn a_manual_merge_needs_no_lease() {
    let w = World::new(&[("1", ""), ("2", "")]);
    std::fs::write(w.clones[0].join("code-1.txt"), "edited, uncommitted\n").unwrap();
    let mut laptop = Daemon::new("laptop", &[0], 0);
    let mut ssu = Daemon::new("ssu-dev", &[1], 1);
    for now in [70, 100] {
        laptop.tick(&w, now).await;
        ssu.tick(&w, now).await;
    }
    assert!(laptop.holds_lease(&w));
    // The owner merges PR 2 by hand on GitHub while the laptop holds the lease.
    git(&w.dir.path().join("wt1"), &["fetch", "-q", "origin"]);
    let merge = std::process::Command::new(w.dir.path().join("gh"))
        .args(["pr", "merge", "2", "--squash"])
        .current_dir(&w.clones[1])
        .output()
        .unwrap();
    assert!(merge.status.success());
    ssu.tick(&w, 130).await;
    assert_eq!(
        ssu.watch(2 - 1).state,
        Phase::Merged,
        "{:?}",
        ssu.watch(1).queue
    );
}

#[tokio::test]
async fn with_sharing_off_the_daemon_never_touches_the_lease() {
    let w = World::new(&[("1", ""), ("2", "")]);
    let mut ws: Vec<Watch> = (0..2)
        .map(|i| Watch::new(&format!("s{i}"), Mode::Merge, "squash", i))
        .collect();
    let gh = w.gh();
    for round in 0..8 {
        for i in 0..2 {
            if !ws[i as usize].active() {
                continue;
            }
            let mut next = ws[i as usize].clone();
            let all = ws.clone();
            advance(
                &Queue::Local,
                &gh,
                &w.clones[i as usize],
                &mut next,
                all,
                true,
                100 + round * 60,
                Some(w.verify()),
            )
            .await;
            ws[i as usize] = next;
        }
    }
    assert!(ws.iter().all(|w| w.state == Phase::Merged));
    assert_eq!(api_calls(&w), 0);
    assert!(ws[0].queue.as_ref().unwrap().lease.is_none());
}
