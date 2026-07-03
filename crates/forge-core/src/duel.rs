//! `/duel`: model arena with routing learning (docs/features/duel.md). Runs the SAME task through
//! up to [`MAX_CANDIDATES`] mesh models concurrently, each in its own isolated git worktree
//! ([`worktree`]) — so the user can pick a winner from real, comparable results (diffstat / test
//! badge / duration / cost) instead of trusting the mesh's single pick blind. The outcome softly
//! biases future routing in THIS repo via `forge_store::Store::record_duel_outcome` /
//! `duel_boosts` (consumed by `forge_mesh::HeuristicRouter::with_repo_boosts`).

use std::path::Path;
use std::time::Instant;

use forge_mesh::BudgetState;
use forge_provider::StreamEvent;
use forge_types::PermissionMode;

use crate::subagent::{self, AgentCtx, Lifecycle, ResolvedAgent};
use crate::{worktree, CoreError};

/// How many models `/duel` races (also the hard cap on the arena size — one worktree per
/// candidate, so this bounds concurrent disk/build load too).
pub const MAX_CANDIDATES: usize = 3;

/// Timeout for a candidate's post-implementation test run, in its own worktree.
const DUEL_TEST_TIMEOUT_SECS: u64 = 120;

const DUEL_SYSTEM_PROMPT: &str = "You are one candidate in a model arena (/duel): implement the \
    task FULLY and correctly in this isolated working copy. Work autonomously — there is no user \
    to ask questions of, so make reasonable judgment calls and note them in your final answer. Do \
    not run anything interactive; keep shell commands non-interactive and bounded.";

/// Tools a duel candidate gets — the full read+write+shell set (unlike a read-only investigation
/// subagent), since it must actually implement the task.
const DUEL_TOOLS: &[&str] = &[
    "read_file",
    "list_dir",
    "search",
    "edit_file",
    "write_file",
    "shell",
];

/// One model's result in a duel — everything the picker needs to render a comparable row.
#[derive(Debug, Clone)]
pub struct DuelCandidate {
    pub model: String,
    pub child_id: String,
    pub branch: String,
    pub files_changed: usize,
    pub added: usize,
    pub removed: usize,
    /// `None` when no test command could be detected in the candidate's worktree.
    pub tests_passed: Option<bool>,
    pub duration_ms: u64,
    pub cost_usd: f64,
    pub summary: String,
    /// Whether the candidate finished cleanly: the agent loop didn't error AND (if tests ran)
    /// they passed.
    pub ok: bool,
}

/// The full result of one `/duel <task>` run, ready for the picker.
#[derive(Debug, Clone)]
pub struct DuelReport {
    pub task: String,
    pub candidates: Vec<DuelCandidate>,
}

/// A message from a candidate's task back to the draining loop below (mirrors `subagent`'s
/// `ChildMsg` shape so the activity panel gets the same live progress feed a plain `spawn_agents`
/// batch does).
enum DuelMsg {
    Progress {
        index: usize,
        snippet: String,
    },
    Done {
        index: usize,
        candidate: DuelCandidate,
    },
}

/// Run a `/duel <task>`: route to up to [`MAX_CANDIDATES`] distinct-provider models, implement the
/// SAME task in each one's own isolated worktree concurrently, and return the comparable report
/// plus the still-alive [`worktree::WorktreeGuard`]s (the caller owns picking a winner — merging
/// it back and dropping every guard afterward — so nothing here tears a worktree down early).
pub async fn run(
    ctx: &AgentCtx,
    parent_id: &str,
    budget: BudgetState,
    task: &str,
    on_event: &mut (dyn FnMut(Lifecycle) + Send),
) -> Result<(DuelReport, Vec<worktree::WorktreeGuard>), CoreError> {
    if !worktree::is_git_repo(&ctx.repo_root) {
        return Err(CoreError::Internal("duel needs a git repo".to_string()));
    }

    let health = ctx.store.current_benched().unwrap_or_default();
    let quota = ctx
        .store
        .current_quota()
        .unwrap_or_default()
        .with_plans(ctx.config.mesh.subscriptions.clone())
        .with_conserve(ctx.config.mesh.subscription_conserve);
    let project = crate::project_context::compute(&ctx.repo_root);
    let decisions = ctx
        .router
        .route_candidates(
            task,
            budget,
            &health,
            &quota,
            None,
            &project,
            MAX_CANDIDATES,
        )
        .await;

    // Defensive dedupe: a `Router` impl could hand back the same model twice.
    let mut seen_models = std::collections::HashSet::new();
    let decisions: Vec<_> = decisions
        .into_iter()
        .filter(|d| seen_models.insert(d.model.clone()))
        .take(MAX_CANDIDATES)
        .collect();
    if decisions.len() < 2 {
        return Err(CoreError::Internal(
            "mesh offered only one usable model — /duel needs at least two to race".to_string(),
        ));
    }

    use tokio::sync::mpsc;
    let (tx, mut rx) = mpsc::unbounded_channel::<DuelMsg>();
    let mode_label = format!("{:?}", ctx.mode);
    let n = decisions.len();
    let mut ids: Vec<String> = vec![String::new(); n];
    let mut guards: Vec<worktree::WorktreeGuard> = Vec::with_capacity(n);

    // Tokio does NOT cascade-cancel tasks spawned inside an aborted task (same hazard
    // `subagent::orchestrate` guards against): if the user interrupts the duel turn, every
    // candidate would keep running — calling models and writing into its worktree — after the
    // run was "cancelled". Hold the handles in a drop-guard that aborts them when this future
    // drops; on normal completion they're already finished and abort() is a no-op.
    struct AbortCandidatesOnDrop(Vec<tokio::task::JoinHandle<()>>);
    impl Drop for AbortCandidatesOnDrop {
        fn drop(&mut self) {
            for handle in &self.0 {
                handle.abort();
            }
        }
    }
    let mut child_tasks = AbortCandidatesOnDrop(Vec::with_capacity(n));

    for (i, decision) in decisions.into_iter().enumerate() {
        let model = decision.model.clone();
        let model_short = model.rsplit_once("::").map_or(model.as_str(), |(_, m)| m);
        let child_id = ctx
            .store
            .create_child_session(".", &mode_label, parent_id)
            .map_err(CoreError::from)?;
        let guard = {
            let repo_root = ctx.repo_root.clone();
            let cid = child_id.clone();
            tokio::task::spawn_blocking(move || worktree::WorktreeGuard::create(&repo_root, &cid))
                .await
                .map_err(|e| {
                    CoreError::Internal(format!("worktree create task failed for {model}: {e}"))
                })?
                .map_err(|e| {
                    CoreError::Internal(format!("worktree create failed for {model}: {e}"))
                })?
        };
        let wt_path = guard.path().to_path_buf();
        let branch = guard.branch().to_string();
        guards.push(guard);
        ids[i] = child_id.clone();

        let resolved = ResolvedAgent {
            name: format!("duel:{model_short}"),
            task: task.to_string(),
            system_prompt: DUEL_SYSTEM_PROMPT.to_string(),
            tools: DUEL_TOOLS.iter().map(|s| s.to_string()).collect(),
            tier: Some(decision.tier),
            pinned_model: Some(model.clone()),
        };
        on_event(Lifecycle::Start {
            id: &child_id,
            agent: &resolved.name,
            task,
            model: &model,
        });

        let child_ctx = AgentCtx {
            worktree_root: Some(wt_path.clone()),
            mode: PermissionMode::AcceptEdits,
            ..ctx.clone()
        };
        let tx = tx.clone();
        let repo_root = ctx.repo_root.clone();

        child_tasks.0.push(tokio::spawn(async move {
            let start = Instant::now();
            let mut on_delta = |ev: StreamEvent| {
                let snippet = match ev {
                    StreamEvent::Text(t) | StreamEvent::Reasoning(t) => t,
                    _ => return,
                };
                let _ = tx.send(DuelMsg::Progress { index: i, snippet });
            };
            let outcome = subagent::run_subagent(
                &child_ctx,
                &child_id,
                &resolved,
                decision,
                budget,
                &mut on_delta,
            )
            .await;
            let (final_text, mut ok) = match outcome {
                Ok(o) => (o.final_text, o.ok),
                Err(e) => (format!("error: duel candidate failed: {e}"), false),
            };

            // Snapshot uncommitted edits onto the candidate's branch — both the diffstat below
            // and the eventual winner merge diff HEAD..branch and would see nothing otherwise.
            let commit_res = {
                let wt_path = wt_path.clone();
                tokio::task::spawn_blocking(move || worktree::commit_worktree(&wt_path)).await
            };
            match commit_res {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("duel worktree snapshot failed for {child_id}: {e}"),
                Err(e) => {
                    tracing::warn!("duel worktree snapshot task failed for {child_id}: {e}")
                }
            }
            let (files_changed, added, removed) = {
                let repo_root = repo_root.clone();
                let branch = branch.clone();
                match tokio::task::spawn_blocking(move || diffstat(&repo_root, &branch)).await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("duel diffstat task failed for {child_id}: {e}");
                        (0, 0, 0)
                    }
                }
            };
            let tests_passed = run_tests_in(&wt_path).await;
            if tests_passed == Some(false) {
                ok = false;
            }

            let candidate = DuelCandidate {
                model,
                child_id: child_id.clone(),
                branch,
                files_changed,
                added,
                removed,
                tests_passed,
                duration_ms: start.elapsed().as_millis() as u64,
                cost_usd: child_ctx.store.session_cost(&child_id).unwrap_or(0.0),
                summary: subagent::summary(&final_text),
                ok,
            };
            let _ = tx.send(DuelMsg::Done {
                index: i,
                candidate,
            });
        }));
    }
    drop(tx);

    let mut slots: Vec<Option<DuelCandidate>> = (0..n).map(|_| None).collect();
    while let Some(msg) = rx.recv().await {
        match msg {
            DuelMsg::Progress { index, snippet } => on_event(Lifecycle::Progress {
                id: &ids[index],
                snippet: &snippet,
            }),
            DuelMsg::Done { index, candidate } => {
                on_event(Lifecycle::Done {
                    id: &candidate.child_id,
                    agent: &format!("duel:{}", candidate.model),
                    ok: candidate.ok,
                    summary: &candidate.summary,
                    cost_usd: candidate.cost_usd,
                });
                slots[index] = Some(candidate);
            }
        }
    }

    let candidates: Vec<DuelCandidate> = slots.into_iter().flatten().collect();
    Ok((
        DuelReport {
            task: task.to_string(),
            candidates,
        },
        guards,
    ))
}

/// Apply the chosen candidate's branch back into the main tree — a thin wrapper over
/// [`worktree::merge_worktree_back`] so callers only need `crate::duel`.
pub fn merge_winner(repo_root: &Path, branch: &str) -> Result<worktree::MergeReport, CoreError> {
    worktree::merge_worktree_back(repo_root, branch)
        .map_err(|e| CoreError::Internal(format!("duel merge failed: {e}")))
}

/// Parse `git diff --shortstat` output into `(files_changed, added, removed)`. Any field the
/// output doesn't mention (e.g. no deletions) is `0`; empty input (no diff at all) is `(0, 0, 0)`.
fn parse_shortstat(s: &str) -> (usize, usize, usize) {
    let mut files = 0usize;
    let mut added = 0usize;
    let mut removed = 0usize;
    for part in s.trim().split(',') {
        let part = part.trim();
        if let Some(n) = part
            .strip_suffix(" file changed")
            .or_else(|| part.strip_suffix(" files changed"))
        {
            files = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part
            .strip_suffix(" insertion(+)")
            .or_else(|| part.strip_suffix(" insertions(+)"))
        {
            added = n.trim().parse().unwrap_or(0);
        } else if let Some(n) = part
            .strip_suffix(" deletion(-)")
            .or_else(|| part.strip_suffix(" deletions(-)"))
        {
            removed = n.trim().parse().unwrap_or(0);
        }
    }
    (files, added, removed)
}

/// Diffstat between `HEAD` and `branch` in `repo_root` — best-effort: a git failure (shouldn't
/// happen; the branch was just created off HEAD) reads as "no changes" rather than erroring the
/// whole duel over a display detail.
fn diffstat(repo_root: &Path, branch: &str) -> (usize, usize, usize) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["diff", "--shortstat", "HEAD", branch])
        .output();
    match out {
        Ok(o) if o.status.success() => parse_shortstat(&String::from_utf8_lossy(&o.stdout)),
        _ => (0, 0, 0),
    }
}

/// Detect a project's test command from its worktree contents (same zero-config detection
/// `Session::detect_project_commands` uses for autofix, but against an arbitrary path instead of
/// the process cwd, since each duel candidate's worktree is a different directory).
fn detect_test_cmd(dir: &Path) -> Option<String> {
    if dir.join("Cargo.toml").exists() {
        return Some("cargo test --workspace 2>&1".to_string());
    }
    if dir.join("package.json").exists() {
        return Some("npm test 2>&1".to_string());
    }
    if dir.join("pyproject.toml").exists() || dir.join("setup.py").exists() {
        return Some("python -m pytest --tb=short -q 2>&1".to_string());
    }
    if dir.join("go.mod").exists() {
        return Some("go test ./... 2>&1".to_string());
    }
    None
}

/// Run the detected test command in `dir` (a candidate's worktree). `None` when no test command
/// could be detected — the picker shows a "–" badge rather than a false pass/fail.
async fn run_tests_in(dir: &Path) -> Option<bool> {
    let cmd = detect_test_cmd(dir)?;
    let out =
        forge_tools::run_shell_command(&cmd, &dir.to_string_lossy(), DUEL_TEST_TIMEOUT_SECS).await;
    Some(!crate::shell_command_failed(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortstat_reads_files_insertions_and_deletions() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 10 insertions(+), 2 deletions(-)\n"),
            (3, 10, 2)
        );
    }

    #[test]
    fn parse_shortstat_handles_no_deletions() {
        assert_eq!(
            parse_shortstat("1 file changed, 2 insertions(+)"),
            (1, 2, 0)
        );
    }

    #[test]
    fn parse_shortstat_handles_no_insertions() {
        assert_eq!(parse_shortstat("1 file changed, 2 deletions(-)"), (1, 0, 2));
    }

    #[test]
    fn parse_shortstat_handles_singular_counts() {
        assert_eq!(
            parse_shortstat("1 file changed, 1 insertion(+), 1 deletion(-)"),
            (1, 1, 1)
        );
    }

    #[test]
    fn parse_shortstat_handles_empty_output() {
        assert_eq!(parse_shortstat(""), (0, 0, 0));
        assert_eq!(parse_shortstat("\n"), (0, 0, 0));
    }

    #[test]
    fn detect_test_cmd_recognizes_cargo_project() {
        let dir = std::env::temp_dir().join(format!("forge-duel-test-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        assert_eq!(
            detect_test_cmd(&dir),
            Some("cargo test --workspace 2>&1".to_string())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_test_cmd_none_for_unrecognized_project() {
        let dir = std::env::temp_dir().join(format!("forge-duel-test-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(detect_test_cmd(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
