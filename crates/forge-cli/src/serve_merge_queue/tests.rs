use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;
use crate::serve_pr::Gh;
use crate::serve_pr_watch::advance;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const CHANGELOG: &str = "# Changelog\n\n## Unreleased\n\n- initial release\n";

/// A bare remote, one clone per PR session and a fake `gh` that answers from the remote.
struct World {
    dir: tempfile::TempDir,
    clones: Vec<PathBuf>,
}

impl World {
    fn bare(&self) -> PathBuf {
        self.dir.path().join("remote.git")
    }

    fn new(prs: &[(&str, &str)]) -> Self {
        Self::build(prs, false)
    }

    /// With `actions`, the seed commit carries a `.forge/config.toml` whose `Verify` action is
    /// `verify.sh`: it logs the tree it ran against and fails on any `broken-*.txt`.
    fn build(prs: &[(&str, &str)], actions: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        let seed = root.join("seed");
        git(root, &["clone", "-q", "remote.git", "seed"]);
        git(&seed, &["config", "user.name", "Seed"]);
        git(&seed, &["config", "user.email", "seed@example.invalid"]);
        git(&seed, &["checkout", "-q", "-b", "main"]);
        std::fs::write(seed.join("CHANGELOG.md"), CHANGELOG).unwrap();
        std::fs::create_dir_all(seed.join("pkg")).unwrap();
        std::fs::write(seed.join("pkg/CHANGELOG.md"), CHANGELOG).unwrap();
        std::fs::write(seed.join("shared.txt"), "shared line\n").unwrap();
        if actions {
            std::fs::create_dir_all(seed.join(".forge")).unwrap();
            std::fs::write(
                seed.join(".forge/config.toml"),
                format!(
                    "[[actions]]\nname = \"Verify\"\ncommand = \"sh verify.sh {}\"\n",
                    root.join("verify.log").display()
                ),
            )
            .unwrap();
            std::fs::write(
                seed.join("verify.sh"),
                "#!/bin/sh\nif ls broken-*.txt >/dev/null 2>&1; then\n  echo 'compiling crate...'\n  echo \"error[E0308]: mismatched types in $(ls broken-*.txt)\" >&2\n  echo 'test result: FAILED. 1 failed' >&2\n  exit 1\nfi\necho \"$(git rev-parse --abbrev-ref HEAD): $(ls code-*.txt | tr '\\n' ' ')\" >> \"$1\"\n",
            )
            .unwrap();
        }
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "seed"]);
        git(&seed, &["push", "-q", "origin", "main"]);

        let mut clones = Vec::new();
        for (n, shared) in prs {
            let name = format!("wt{n}");
            git(root, &["clone", "-q", "remote.git", &name]);
            let wt = root.join(&name);
            git(&wt, &["config", "user.name", &format!("Session {n}")]);
            git(
                &wt,
                &["config", "user.email", &format!("s{n}@example.invalid")],
            );
            git(&wt, &["checkout", "-q", "-b", &format!("feat-{n}")]);
            let entry =
                format!("# Changelog\n\n## Unreleased\n\n- pr {n} entry\n- initial release\n");
            std::fs::write(wt.join("CHANGELOG.md"), &entry).unwrap();
            std::fs::write(wt.join("pkg/CHANGELOG.md"), &entry).unwrap();
            std::fs::write(wt.join(format!("code-{n}.txt")), format!("code {n}\n")).unwrap();
            if !shared.is_empty() {
                std::fs::write(wt.join("shared.txt"), format!("{shared}\n")).unwrap();
            }
            git(&wt, &["add", "."]);
            git(&wt, &["commit", "-q", "-m", &format!("feat {n}")]);
            git(&wt, &["push", "-q", "origin", &format!("feat-{n}")]);
            clones.push(wt);
        }
        let world = Self { dir, clones };
        world.install_gh();
        world
    }

    fn install_gh(&self) {
        let script = format!(
            r#"#!/bin/sh
echo "$@" >> "{log}"
BARE="{bare}"
case "$1 $2" in
"pr view")
  br=$(git rev-parse --abbrev-ref HEAD)
  n=${{br#feat-}}
  head=$(git --git-dir="$BARE" rev-parse "refs/heads/$br")
  if git --git-dir="$BARE" merge-base --is-ancestor "$head" refs/heads/main; then st=MERGED; ms=CLEAN
  elif git --git-dir="$BARE" merge-base --is-ancestor refs/heads/main "$head"; then st=OPEN; ms=CLEAN
  else st=OPEN; ms=BEHIND; fi
  printf '{{"number":%s,"url":"https://example.invalid/pr/%s","title":"t","state":"%s","isDraft":false,"headRefName":"%s","baseRefName":"main","headRefOid":"%s","mergeable":"MERGEABLE","mergeStateStatus":"%s","reviewDecision":"","statusCheckRollup":[{{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}}],"comments":[],"reviews":[]}}' "$n" "$n" "$st" "$br" "$head" "$ms"
  ;;
"pr merge")
  n=$3
  m="{tmp}/merge-$n"
  rm -rf "$m"
  git clone -q "$BARE" "$m" || exit 1
  cd "$m" || exit 1
  git config user.name Merger; git config user.email merger@example.invalid
  git merge -q --no-ff -m "Merge PR $n" "origin/feat-$n" || exit 1
  git push -q origin HEAD:refs/heads/main || exit 1
  ;;
*) echo "unexpected gh call: $*" >&2; exit 1 ;;
esac
"#,
            log = self.dir.path().join("gh.log").display(),
            bare = self.bare().display(),
            tmp = self.dir.path().display(),
        );
        let path = self.dir.path().join("gh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn gh(&self) -> Gh {
        Gh::with_program(self.dir.path().join("gh"))
    }

    fn gh_log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("gh.log")).unwrap_or_default()
    }

    fn verify_log(&self) -> PathBuf {
        self.dir.path().join("verify.log")
    }

    /// Records which PRs' code is in the tree it verifies, and refuses leftover markers or
    /// diverged changelog copies.
    fn verify(&self) -> Vec<Verify> {
        vec![Verify {
            name: "verify".into(),
            command: format!(
                "! grep -rq '^<<<<<<<' CHANGELOG.md pkg/CHANGELOG.md && cmp -s CHANGELOG.md pkg/CHANGELOG.md \
                 && echo \"$(git rev-parse --abbrev-ref HEAD): $(ls code-*.txt | tr '\\n' ' ')\" >> {}",
                self.verify_log().display()
            ),
            cwd: None,
        }]
    }

    /// The daemon's sweep, without the daemon: every active watch gets its turn each minute.
    async fn run(&self, watches: &mut [Watch], rounds: i64, start: i64) -> Vec<(usize, String)> {
        let gh = self.gh();
        let mut prompts = Vec::new();
        for round in 0..rounds {
            let now = start + round * 60;
            for i in 0..watches.len() {
                if !watches[i].active() {
                    continue;
                }
                let mut next = watches[i].clone();
                let all = watches.to_vec();
                let out = advance(
                    &gh,
                    &self.clones[i],
                    &mut next,
                    all,
                    true,
                    now,
                    Some(self.verify()),
                )
                .await;
                watches[i] = next;
                println!(
                    "round {round} session s{i} {:?} {:?} hold={:?}",
                    watches[i].state,
                    watches[i]
                        .queue
                        .as_ref()
                        .map(|q| (q.state, q.reason.clone())),
                    watches[i].hold
                );
                if let Some(p) = out.prompt {
                    prompts.push((i, p));
                }
            }
        }
        prompts
    }
}

fn watches(n: usize) -> Vec<Watch> {
    (0..n)
        .map(|i| {
            Watch::new(
                &format!("s{i}"),
                Mode::Merge,
                "squash",
                i64::try_from(i).unwrap(),
            )
        })
        .collect()
}

fn main_file(w: &World, path: &str) -> String {
    git(&w.bare(), &["show", &format!("main:{path}")])
}

/// PR4's session resolves its conflict by hand and pushes: the PR re-enters and lands last.
fn resolve_four_by_hand(w: &World) {
    let wt = &w.clones[1];
    let merge = Command::new("git")
        .arg("-C")
        .arg(wt)
        .args(["merge", "origin/main"])
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the hand-off was a real conflict");
    // (the queue aborted its own attempt; this is the session's attempt)
    git(wt, &["fetch", "-q", "origin"]);
    let _ = Command::new("git")
        .arg("-C")
        .arg(wt)
        .args(["merge", "--abort"])
        .output();
    let _ = Command::new("git")
        .arg("-C")
        .arg(wt)
        .args(["merge", "origin/main"])
        .output();
    std::fs::write(wt.join("shared.txt"), "from one and four\n").unwrap();
    for path in ["CHANGELOG.md", "pkg/CHANGELOG.md"] {
        let text = main_file(w, path).replace("- pr 1 entry\n", "- pr 1 entry\n- pr 4 entry\n");
        std::fs::write(wt.join(path), text).unwrap();
    }
    git(wt, &["add", "."]);
    git(wt, &["commit", "-q", "--no-edit"]);
    git(wt, &["push", "-q", "origin", "feat-4"]);
}

#[tokio::test]
async fn the_nights_prs_land_one_at_a_time_and_a_real_conflict_does_not_block_the_queue() {
    // Queue order: PR1, PR4 (will conflict with PR1 in shared.txt), PR2, PR3.
    let w = World::new(&[("1", "from one"), ("4", "from four"), ("2", ""), ("3", "")]);
    let mut ws = watches(4);
    let prompts = w.run(&mut ws, 14, 1000).await;

    // PR4 was handed to its session with the path named, and sits skipped.
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].0, 1);
    assert!(prompts[0].1.contains("- shared.txt"), "{}", prompts[0].1);
    assert!(prompts[0].1.contains("never use `gh pr update-branch`"));
    let q4 = ws[1].queue.as_ref().unwrap();
    assert_eq!(q4.state, QueueState::Skipped);
    assert!(q4.reason.as_deref().unwrap().contains("shared.txt"));
    assert!(ws[1].active(), "the skipped PR keeps its watch");

    // The others landed, serially, in order, each verified against the main before it.
    for i in [0, 2, 3] {
        assert_eq!(ws[i].state, Phase::Merged, "watch {i}: {:?}", ws[i].queue);
    }
    let log = w.gh_log();
    let merges: Vec<&str> = log.lines().filter(|l| l.starts_with("pr merge")).collect();
    assert_eq!(merges.len(), 3, "{log}");
    assert!(merges[0].starts_with("pr merge 1 "), "{log}");
    assert!(merges[1].starts_with("pr merge 2 "), "{log}");
    assert!(merges[2].starts_with("pr merge 3 "), "{log}");
    assert!(!log.contains("update-branch"), "{log}");

    let verified = std::fs::read_to_string(w.verify_log()).unwrap();
    let verified: Vec<&str> = verified.lines().collect();
    assert_eq!(
        verified.len(),
        2,
        "PR1 was current and needed no update: {verified:?}"
    );
    assert!(verified[0].starts_with("feat-2:") && verified[0].contains("code-1.txt"));
    assert!(verified[0].contains("code-2.txt"));
    assert!(verified[1].starts_with("feat-3:"));
    assert!(verified[1].contains("code-1.txt") && verified[1].contains("code-2.txt"));
    assert!(verified[1].contains("code-3.txt"));

    assert_eq!(ws[2].queue.as_ref().unwrap().pushes, 1);
    assert_eq!(ws[3].queue.as_ref().unwrap().pushes, 1);
    assert!(ws[2]
        .queue
        .as_ref()
        .unwrap()
        .last_update
        .as_deref()
        .unwrap()
        .contains("resolved CHANGELOG.md, pkg/CHANGELOG.md"));

    // The merge commits are the sessions' own, never a bot's.
    assert_eq!(
        git(&w.clones[2], &["log", "-1", "--format=%an <%ae>", "feat-2"]),
        "Session 2 <s2@example.invalid>"
    );
    assert_eq!(
        git(&w.clones[3], &["log", "-1", "--format=%an", "feat-3"]),
        "Session 3"
    );

    // Main has every entry, once, in both copies, and no markers.
    for path in ["CHANGELOG.md", "pkg/CHANGELOG.md"] {
        let text = main_file(&w, path);
        for n in [1, 2, 3] {
            assert_eq!(
                text.matches(&format!("- pr {n} entry")).count(),
                1,
                "{path}:\n{text}"
            );
        }
        assert_eq!(text.matches("- initial release").count(), 1);
        assert!(!text.contains("<<<<<<<") && !text.contains(">>>>>>>"));
    }
    assert_eq!(
        main_file(&w, "CHANGELOG.md"),
        main_file(&w, "pkg/CHANGELOG.md")
    );
    for n in [1, 2, 3] {
        assert_eq!(main_file(&w, &format!("code-{n}.txt")), format!("code {n}"));
    }
    assert_eq!(main_file(&w, "shared.txt"), "from one");

    println!("--- remote main after the queue ---");
    println!(
        "{}",
        git(
            &w.bare(),
            &["log", "--graph", "--format=%h %an: %s", "main"]
        )
    );
    println!(
        "--- CHANGELOG.md on main ---\n{}",
        main_file(&w, "CHANGELOG.md")
    );
    println!("--- hand-off prompt for PR 4 ---\n{}", prompts[0].1);
    println!("--- verification log ---\n{}", verified.join("\n"));
    println!(
        "--- gh calls that changed anything ---\n{}",
        merges.join("\n")
    );
    // The snapshot a surface shows.
    let snap = snapshot(&ws);
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].rows[0].state, "skipped");
    assert_eq!(snap[0].rows[0].position, None);
    assert_eq!(
        snap[0].rows.iter().filter(|r| r.state == "merged").count(),
        3
    );
    assert_eq!(
        brief_line(&ws).unwrap().line,
        "Merge queue: 0 waiting, 1 handed back"
    );

    resolve_four_by_hand(&w);
    w.run(&mut ws, 6, 5000).await;
    assert_eq!(ws[1].state, Phase::Merged, "{:?}", ws[1].queue);
    assert_eq!(main_file(&w, "shared.txt"), "from one and four");
    assert!(main_file(&w, "CHANGELOG.md").contains("- pr 4 entry"));
}

#[tokio::test]
async fn a_dirty_worktree_waits_and_a_non_head_never_merges() {
    let w = World::new(&[("1", ""), ("2", "")]);
    std::fs::write(w.clones[0].join("code-1.txt"), "edited, uncommitted\n").unwrap();
    let mut ws = watches(2);
    w.run(&mut ws, 3, 100).await;
    assert!(ws[0].active() && ws[1].active());
    assert_eq!(ws[0].queue.as_ref().unwrap().state, QueueState::Head);
    assert_eq!(ws[1].queue.as_ref().unwrap().state, QueueState::Queued);
    assert!(ws[1]
        .hold
        .as_deref()
        .unwrap()
        .contains("#2 in the merge queue"));
    assert!(!w.gh_log().contains("pr merge"), "{}", w.gh_log());
    std::fs::write(w.clones[0].join("code-1.txt"), "code 1\n").unwrap();
    w.run(&mut ws, 8, 1000).await;
    assert_eq!(ws[0].state, Phase::Merged);
    assert_eq!(ws[1].state, Phase::Merged);
}

#[tokio::test]
async fn an_unresolvable_changelog_conflict_is_a_hand_off_not_a_guess() {
    let w = World::new(&[("1", ""), ("2", "")]);
    // PR1 rewrites the existing entry, PR2 prepends: not a pure append, so no mechanical answer.
    let wt = &w.clones[0];
    for path in ["CHANGELOG.md", "pkg/CHANGELOG.md"] {
        std::fs::write(
            wt.join(path),
            "# Changelog\n\n## Unreleased\n\n- pr 1 entry\n- initial release, reworded\n",
        )
        .unwrap();
    }
    git(wt, &["commit", "-qam", "reword"]);
    git(wt, &["push", "-q", "origin", "feat-1"]);
    let mut ws = watches(2);
    let prompts = w.run(&mut ws, 12, 100).await;
    assert_eq!(ws[0].state, Phase::Merged);
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].1.contains("- CHANGELOG.md"), "{}", prompts[0].1);
    assert_eq!(ws[1].queue.as_ref().unwrap().state, QueueState::Skipped);
    assert!(
        git(&w.clones[1], &["status", "--porcelain"]).is_empty(),
        "the attempt was aborted"
    );
    assert!(!main_file(&w, "CHANGELOG.md").contains("<<<<<<<"));
}

#[tokio::test]
async fn a_failing_verification_keeps_the_push_back_and_tells_the_session() {
    let w = World::new(&[("1", ""), ("2", "")]);
    let mut ws = watches(2);
    let gh = w.gh();
    let failing = vec![Verify {
        name: "verify".into(),
        command: "echo boom-in-merged-result >&2; exit 1".into(),
        cwd: None,
    }];
    let mut prompt = None;
    for round in 0..14 {
        let now = 100 + round * 60;
        for i in 0..2 {
            if !ws[i].active() {
                continue;
            }
            let mut next = ws[i].clone();
            let verify = if i == 1 { failing.clone() } else { w.verify() };
            let all = ws.to_vec();
            let out = advance(&gh, &w.clones[i], &mut next, all, true, now, Some(verify)).await;
            ws[i] = next;
            if i == 1 && out.prompt.is_some() {
                prompt = out.prompt;
            }
        }
    }
    let prompt = prompt.expect("PR2 is told its verification failed");
    assert!(prompt.contains("boom-in-merged-result"), "{prompt}");
    assert!(ws[1]
        .queue
        .as_ref()
        .unwrap()
        .reason
        .as_deref()
        .unwrap()
        .contains("verification"));
    let remote = git(&w.bare(), &["rev-parse", "refs/heads/feat-2"]);
    assert_ne!(
        remote,
        git(&w.clones[1], &["rev-parse", "HEAD"]),
        "nothing was pushed"
    );
}

#[test]
fn verify_comes_from_the_projects_verify_and_check_actions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
    std::fs::write(
        dir.path().join(".forge/config.toml"),
        "[[actions]]\nname = \"dev\"\ncommand = \"npm run dev\"\n[[actions]]\nname = \"Verify\"\ncommand = \"make test\"\n",
    )
    .unwrap();
    std::env::set_var("FORGE_TRUST_PROJECT", "1");
    let found = verify_for(dir.path());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].command, "make test");
}

fn act(ws: &mut [Watch], session: &str, action: Action) -> Result<String, ActionError> {
    let mut map: std::collections::HashMap<String, Watch> = ws
        .iter()
        .map(|w| (w.session_id.clone(), w.clone()))
        .collect();
    let result = apply(&mut map, session, action);
    for w in ws.iter_mut() {
        *w = map[&w.session_id].clone();
    }
    result
}

fn merged_order(w: &World) -> Vec<String> {
    w.gh_log()
        .lines()
        .filter(|l| l.starts_with("pr merge"))
        .map(|l| l.split_whitespace().nth(2).unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_held_pr_neither_lands_nor_blocks_the_queue_and_release_puts_it_back() {
    let w = World::new(&[("1", ""), ("2", "")]);
    // PR1 is the head but cannot move yet (uncommitted work), so the hold lands on a live queue.
    std::fs::write(w.clones[0].join("code-1.txt"), "edited, uncommitted\n").unwrap();
    let mut ws = watches(2);
    w.run(&mut ws, 2, 100).await;
    assert_eq!(ws[0].queue.as_ref().unwrap().state, QueueState::Head);

    assert!(
        act(&mut ws, "s0", Action::Release).is_err(),
        "nothing to release yet"
    );
    act(&mut ws, "s0", Action::Hold).unwrap();
    assert!(matches!(
        act(&mut ws, "s0", Action::Hold),
        Err(ActionError::Conflict(_))
    ));
    w.run(&mut ws, 6, 1000).await;
    assert_eq!(
        ws[1].state,
        Phase::Merged,
        "PR2 is not stuck behind a held PR"
    );
    assert_eq!(ws[0].state, Phase::Watching);
    assert_eq!(ws[0].queue.as_ref().unwrap().state, QueueState::Held);
    assert_eq!(merged_order(&w), vec!["2"]);
    let snap = snapshot(&ws);
    assert_eq!(snap[0].rows[0].state, "held");
    assert!(snap[0].rows[0].held && snap[0].rows[0].position.is_none());
    assert_eq!(
        brief_line(&ws).unwrap().line,
        "Merge queue: 0 waiting, 1 held"
    );

    std::fs::write(w.clones[0].join("code-1.txt"), "code 1\n").unwrap();
    act(&mut ws, "s0", Action::Release).unwrap();
    w.run(&mut ws, 8, 5000).await;
    assert_eq!(ws[0].state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["2", "1"]);
}

#[tokio::test]
async fn front_makes_a_later_pr_the_next_to_land() {
    let w = World::new(&[("1", ""), ("2", ""), ("3", "")]);
    std::fs::write(w.clones[0].join("code-1.txt"), "edited, uncommitted\n").unwrap();
    let mut ws = watches(3);
    w.run(&mut ws, 2, 100).await;
    let place = |ws: &[Watch], pr: &str| {
        snapshot(ws)[0]
            .rows
            .iter()
            .find(|r| r.session_id == pr)
            .and_then(|r| r.position)
    };
    assert_eq!((place(&ws, "s0"), place(&ws, "s2")), (Some(1), Some(3)));
    assert!(act(&mut ws, "s0", Action::Front)
        .unwrap()
        .contains("already at the front"));

    act(&mut ws, "s2", Action::Front).unwrap();
    assert_eq!((place(&ws, "s2"), place(&ws, "s0")), (Some(1), Some(2)));
    w.run(&mut ws, 4, 1000).await;
    assert_eq!(
        merged_order(&w),
        vec!["3"],
        "PR3 jumped the line; PR1 still blocks PR2"
    );

    std::fs::write(w.clones[0].join("code-1.txt"), "code 1\n").unwrap();
    w.run(&mut ws, 6, 5000).await;
    assert_eq!(merged_order(&w), vec!["3", "1", "2"]);
}

#[tokio::test]
async fn retry_re_enters_a_handed_back_pr_without_a_new_push() {
    let w = World::new(&[("1", "from one"), ("4", "from four")]);
    let mut ws = watches(2);
    let prompts = w.run(&mut ws, 6, 100).await;
    assert_eq!(prompts.len(), 1);
    assert_eq!(ws[1].queue.as_ref().unwrap().state, QueueState::Skipped);
    assert!(
        act(&mut ws, "s0", Action::Retry).is_err(),
        "a merged PR has no queue to re-enter"
    );

    act(&mut ws, "s1", Action::Retry).unwrap();
    assert_eq!(ws[1].queue.as_ref().unwrap().state, QueueState::Queued);
    let again = w.run(&mut ws, 4, 1000).await;
    assert_eq!(
        again.len(),
        1,
        "the retry ran the update step again: {again:?}"
    );
    assert_eq!(ws[1].queue.as_ref().unwrap().state, QueueState::Skipped);
    assert!(matches!(
        act(&mut ws, "s1", Action::Hold).map(|_| ()),
        Ok(())
    ));
    assert!(matches!(
        act(&mut ws, "s1", Action::Retry),
        Err(ActionError::Conflict(_))
    ));
}

#[test]
fn actions_find_a_pr_by_number_and_refuse_an_ambiguous_one() {
    let mut a = watches(2);
    for (i, w) in a.iter_mut().enumerate() {
        w.pr_number = Some(7);
        let mut info = QueueInfo::new(format!("repo-{i} @ main"));
        info.state = QueueState::Queued;
        w.queue = Some(info);
    }
    let map: std::collections::HashMap<String, Watch> = a
        .iter()
        .map(|w| (w.session_id.clone(), w.clone()))
        .collect();
    assert!(matches!(
        resolve(&map, None, Some(7)),
        Err(ActionError::Conflict(m)) if m.contains("s0, s1")
    ));
    assert!(matches!(
        resolve(&map, None, Some(8)),
        Err(ActionError::NotFound(_))
    ));
    assert_eq!(resolve(&map, Some("s1"), None).unwrap(), "s1");
    assert!(matches!(
        resolve(&map, Some("nope"), Some(7)),
        Err(ActionError::NotFound(_))
    ));
    let mut one = map.clone();
    one.remove("s1");
    assert_eq!(resolve(&one, None, Some(7)).unwrap(), "s0");
}

#[test]
fn what_the_owner_chose_while_a_poll_ran_survives_storing_its_result() {
    let mut live = watches(1).remove(0);
    let mut info = QueueInfo::new("r @ main".into());
    info.held = true;
    info.rank = Some(-3);
    info.retry_seq = 2;
    live.queue = Some(info);
    let mut stale = live.clone();
    let q = stale.queue.as_mut().unwrap();
    q.held = false;
    q.rank = None;
    q.retry_seq = 0;
    q.pushes = 5;
    carry_owner_edits(&live, &mut stale);
    let q = stale.queue.unwrap();
    assert!(q.held && q.rank == Some(-3) && q.retry_seq == 2);
    assert_eq!(q.pushes, 5, "the poll's own work is kept");
}

// ---- the daemon's sweep, end to end

use crate::serve::tests::FORGE_DB_LOCK;
use crate::serve::{start_session, CreateSessionReq, DaemonState, SessionRegistry};
use crate::serve_pr_watch::{registry, sweep};

fn mock_state(dir: &Path) -> std::sync::Arc<DaemonState> {
    std::sync::Arc::new(DaemonState {
        registry: std::sync::Arc::new(SessionRegistry::new()),
        terminals: std::sync::Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store: std::sync::Arc::new(crate::open_store().unwrap()),
        base: "/tok".into(),
        mock: true,
        default_cwd: dir.display().to_string(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    })
}

/// One daemon session per clone, each with a merge-mode watch armed in clone order.
async fn arm_sessions(state: &DaemonState, w: &World) -> Vec<String> {
    let mut ids = Vec::new();
    for (i, clone) in w.clones.iter().enumerate() {
        let handle = start_session(
            state,
            CreateSessionReq {
                cwd: Some(clone.display().to_string()),
                ..Default::default()
            },
        )
        .await
        .ok()
        .unwrap();
        registry().arm(&handle.session_id, Mode::Merge, "squash", 100 + i as i64);
        ids.push(handle.session_id.clone());
    }
    ids
}

fn watch_of(id: &str) -> Watch {
    registry().get(id).expect("armed")
}

async fn sweeps(state: &std::sync::Arc<DaemonState>, w: &World, rounds: i64, start: i64) {
    let gh = w.gh();
    for round in 0..rounds {
        sweep(state, &gh, start + round * 60).await;
    }
}

fn disarm(ids: &[String]) {
    for id in ids {
        registry().arm(id, Mode::Off, "squash", 0);
    }
    std::env::remove_var("FORGE_DB");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_daemon_sweep_lands_the_nights_prs_verifying_through_the_projects_actions() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::build(
        &[("1", "from one"), ("4", "from four"), ("2", ""), ("3", "")],
        true,
    );
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    std::env::set_var("FORGE_TRUST_PROJECT", "1");
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;

    sweeps(&state, &w, 14, 1000).await;

    for i in [0, 2, 3] {
        assert_eq!(
            watch_of(&ids[i]).state,
            Phase::Merged,
            "{:?}",
            watch_of(&ids[i]).queue
        );
    }
    assert_eq!(merged_order(&w), vec!["1", "2", "3"]);
    let four = watch_of(&ids[1]);
    let q = four.queue.as_ref().unwrap();
    assert_eq!(q.state, QueueState::Skipped);
    assert!(q.reason.as_deref().unwrap().contains("shared.txt"));
    assert!(four
        .last_action
        .as_deref()
        .unwrap()
        .starts_with("handed to its session"));

    // The project's own `Verify` action ran on the merged result of PR2 and PR3, not PR1.
    let log = std::fs::read_to_string(w.verify_log()).unwrap();
    let log: Vec<&str> = log.lines().collect();
    assert_eq!(log.len(), 2, "{log:?}");
    assert!(log[0].starts_with("feat-2:") && log[0].contains("code-1.txt"));
    assert!(log[1].starts_with("feat-3:") && log[1].contains("code-2.txt"));
    assert!(watch_of(&ids[2])
        .queue
        .unwrap()
        .last_update
        .unwrap()
        .contains("verified: Verify"));

    // The session resolves PR4 by hand; the sweep takes it back and lands it last.
    resolve_four_by_hand(&w);
    sweeps(&state, &w, 6, 5000).await;
    assert_eq!(watch_of(&ids[1]).state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["1", "2", "3", "4"]);
    assert_eq!(main_file(&w, "shared.txt"), "from one and four");
    disarm(&ids);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_verify_action_skips_the_pr_with_the_output_tail_and_retry_runs_it_again() {
    let _env = FORGE_DB_LOCK.lock().await;
    let w = World::build(&[("1", ""), ("2", ""), ("3", "")], true);
    std::env::set_var("FORGE_DB", w.dir.path().join("forge.db"));
    std::env::set_var("FORGE_TRUST_PROJECT", "1");
    // PR2's branch breaks the project's verification once the base is merged into it.
    let wt = &w.clones[1];
    std::fs::write(wt.join("broken-2.txt"), "x\n").unwrap();
    git(wt, &["add", "."]);
    git(wt, &["commit", "-q", "-m", "break"]);
    git(wt, &["push", "-q", "origin", "feat-2"]);
    let state = mock_state(w.dir.path());
    let ids = arm_sessions(&state, &w).await;

    sweeps(&state, &w, 12, 1000).await;

    assert_eq!(watch_of(&ids[0]).state, Phase::Merged);
    assert_eq!(
        watch_of(&ids[2]).state,
        Phase::Merged,
        "PR3 is not stuck behind the broken PR"
    );
    assert_eq!(merged_order(&w), vec!["1", "3"]);
    let two = watch_of(&ids[1]);
    assert_eq!(two.state, Phase::Watching);
    let q = two.queue.as_ref().unwrap();
    assert_eq!(q.state, QueueState::Skipped);
    let reason = q.reason.as_deref().unwrap();
    assert!(
        reason.starts_with("verification `Verify` failed: "),
        "{reason}"
    );
    assert!(
        reason.contains("error[E0308]: mismatched types in broken-2.txt"),
        "{reason}"
    );
    assert!(reason.contains("test result: FAILED. 1 failed"), "{reason}");
    assert_eq!(
        git(&w.bare(), &["rev-parse", "refs/heads/feat-2"]),
        git(wt, &["rev-parse", "origin/feat-2"]),
        "nothing was pushed"
    );
    let row = &snapshot(&registry().all())
        .into_iter()
        .flat_map(|q| q.rows)
        .find(|r| r.session_id == ids[1])
        .unwrap();
    assert_eq!(row.state, "skipped");
    assert!(row.reason.as_deref().unwrap().contains("FAILED"));

    // Retry by PR number, without touching the branch: the verification runs again and fails again.
    registry()
        .queue_action(None, two.pr_number, Action::Retry)
        .unwrap();
    assert_eq!(watch_of(&ids[1]).queue.unwrap().state, QueueState::Queued);
    sweeps(&state, &w, 3, 5000).await;
    assert_eq!(watch_of(&ids[1]).queue.unwrap().state, QueueState::Skipped);

    // The session fixes it and pushes: the PR re-enters on its own and lands.
    git(wt, &["rm", "-q", "broken-2.txt"]);
    git(wt, &["commit", "-q", "-m", "fix"]);
    git(wt, &["push", "-q", "origin", "feat-2"]);
    sweeps(&state, &w, 8, 9000).await;
    assert_eq!(watch_of(&ids[1]).state, Phase::Merged);
    assert_eq!(merged_order(&w), vec!["1", "3", "2"]);
    disarm(&ids);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_queue_routes_list_and_act_with_the_right_statuses() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let _env = FORGE_DB_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("FORGE_DB", dir.path().join("forge.db"));
    let router = crate::serve::daemon_router(mock_state(dir.path()));
    let ids = ["route-a", "route-b"];
    for (i, id) in ids.iter().enumerate() {
        registry().arm(id, Mode::Merge, "squash", 10 + i as i64);
        let mut w = watch_of(id);
        w.pr_number = Some(40 + i as u64);
        let mut info = QueueInfo::new("origin @ main".into());
        info.state = if i == 0 {
            QueueState::Head
        } else {
            QueueState::Queued
        };
        w.queue = Some(info);
        registry().store(id, w);
    }
    let call = |method: &'static str, path: &'static str, body: serde_json::Value| {
        let router = router.clone();
        async move {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };

    let (status, body) = call("GET", "/tok/api/pr/queue", serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let mine: Vec<&serde_json::Value> = body["queues"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|q| q["rows"].as_array().unwrap())
        .filter(|r| ids.contains(&r["session_id"].as_str().unwrap()))
        .collect();
    assert_eq!(mine.len(), 2);

    let act = |action: &str, who: serde_json::Value| {
        let mut body = serde_json::json!({ "action": action });
        body.as_object_mut()
            .unwrap()
            .extend(who.as_object().unwrap().clone());
        body
    };
    let (status, body) = call(
        "POST",
        "/tok/api/pr/queue/action",
        act("hold", serde_json::json!({"session": "route-a"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert!(body["message"].as_str().unwrap().contains("PR #40 held"));
    let held = body["queues"][0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["session_id"] == "route-a")
        .unwrap()
        .clone();
    assert_eq!(
        (held["state"].as_str(), held["held"].as_bool()),
        (Some("held"), Some(true))
    );

    let (status, body) = call(
        "POST",
        "/tok/api/pr/queue/action",
        act("hold", serde_json::json!({"pr": 40})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("already held"));
    let (status, _) = call(
        "POST",
        "/tok/api/pr/queue/action",
        act("front", serde_json::json!({"session": "route-nope"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        "POST",
        "/tok/api/pr/queue/action",
        serde_json::json!({"session": "route-a", "action": "dance"}),
    )
    .await;
    assert!(status.is_client_error());
    let (status, body) = call(
        "POST",
        "/tok/api/pr/queue/action",
        act("release", serde_json::json!({"pr": 40})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    disarm(&ids.map(String::from));
}
