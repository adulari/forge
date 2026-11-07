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

    // The session resolves its conflict by hand and pushes: the PR re-enters and lands last.
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
        let text = main_file(&w, path).replace("- pr 1 entry\n", "- pr 1 entry\n- pr 4 entry\n");
        std::fs::write(wt.join(path), text).unwrap();
    }
    git(wt, &["add", "."]);
    git(wt, &["commit", "-q", "--no-edit"]);
    git(wt, &["push", "-q", "origin", "feat-4"]);
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
