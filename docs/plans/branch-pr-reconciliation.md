# Branch / PR reconciliation

**Status:** active execution record. Refresh this inventory before each landing or cleanup action. This document is intentionally compact: it keeps evidence, ownership, dependencies, dispositions, gates, and commands, not raw logs or generated Cargo metadata.

## Non-negotiable safety rules

- Preserve every unique committed, staged, and unstaged change until it is either landed, explicitly transferred with proof, or assigned an active owner.
- Never reset, clean, force-push, or otherwise alter the root worktree `/home/floris/Documents/Repositories/Personal/AI/forge` on dirty branch `pr/oauth`.
- Never delete a worktree/branch with unique uncommitted work or without recording its path, status, SHA, PR mapping, and comparison with `origin/main`.
- Do not use this planning worktree as a repair base. New repair branches start from a clean worktree at the current `origin/main`.
- PR #769 stays automerge-disabled until P0 `fix/mesh-session-cwd` has landed and its post-merge checks are green.
- Every repaired/replacement PR requires an independent review and current required checks before squash automerge.

## Evidence snapshot (2026-07-15)

- Repository: `adulari/forge`; default branch `main`; squash merge enabled, merge commits disabled, delete-branch-on-merge enabled.
- `origin/main`: `e4b811fd` (`fix(mesh): stabilize concurrent routing and failover (#766)`).
- Open PRs: #769, #759, #739, #738, #737, #735. All have no recorded approval; none has automerge enabled.
- #769 (`fix/reconcile-rmcp-sse-compat`, head `b29cb14b`) is mergeable but blocked while CI is still running; fmt/audit/deny passed, clippy/test/release were queued or in progress at capture. It contains the repair commit on top of #759's dependency commit and must remain disabled until P0.
- #759 (`dependabot/cargo/cargo-minor-patch-3d808effb0`) is blocked: clippy, Ubuntu test, release-build, and aggregate CI failed; its changes include root `Cargo.lock` and must be serialized.
- #739 (`dependabot/cargo/tokio-tungstenite-0.30.0`), #738 (`dependabot/cargo/tower-http-0.7.0`), and #737 (`dependabot/cargo/governor-0.10.4`) are unstable with audit/deny failures and each changes the lockfile/manifests.
- #735 (`dependabot/github_actions/actions/setup-node-6`) is independent of Cargo but currently unstable and has no approval.
- P0 worktree `/tmp/forge-mesh-session-cwd`, branch `fix/mesh-session-cwd`, is at `e4b811fd` with substantial unstaged CLI/core/tools changes and two new core isolation tests. It has no committed delta yet; preserve it exactly and make it the first landing candidate.
- `/tmp/forge-mesh-wave2-a1` / `fix/mesh-postcheck-scope` is also dirty at `e4b811fd`; preserve independently.
- `/tmp/forge-oauth-reconcile` / `fix/oauth-headless` is dirty at `e4b811fd` with OAuth CLI/config changes and a new `oauth_flow.rs`; preserve independently. The root `pr/oauth` contains related dirty WIP plus untracked handoff docs.
- `/tmp/forge-reconcile-rmcp` / `fix/reconcile-rmcp-sse-compat` has a dirty Cargo.lock and CLI manifest, while the branch head is `b29cb14b`; do not discard the dirty state.
- Existing committed mesh/serve work includes `fix/mesh-p0-integration` (`0e199678` and ancestors), `fix/serve-interrupt-queue` (`dd820b7d`), `fix/cli-bridge-permissions` (`ba58afe3`), and `fix/mesh-routing-stability-pr` (`fed47f36`); compare and preserve until proven integrated/superseded.
- The repository contains many `.claude`, `.forge`, workflow, and other agent worktrees. Treat all as owned/unknown until individually checked; do not bulk-clean.

## Explicit active allowlist

Always preserve:

1. Root `/home/floris/Documents/Repositories/Personal/AI/forge`, dirty `pr/oauth`.
2. `/tmp/forge-mesh-session-cwd`, `fix/mesh-session-cwd` (P0; first landing).
3. `forge/subagent/713337ff-111` and its worktree until this reconciliation plan is integrated.
4. `/tmp/forge-mesh-wave2-a1`, `fix/mesh-postcheck-scope`.
5. `/tmp/forge-oauth-reconcile`, `fix/oauth-headless`.
6. `/tmp/forge-reconcile-rmcp`, `fix/reconcile-rmcp-sse-compat`, until its dirty state is reconciled.
7. `/tmp/forge-mesh-p0-integration`, `/tmp/forge-mesh-investigate`, `/tmp/forge-mesh-stability`, and their nested worktrees until their unique commits are compared and dispositioned.

Extend this list whenever inspection proves an owner is actively using another worktree. No allowlisted item may be deleted by automation.

## Disposition table

| Item | Current disposition | Required action |
|---|---|---|
| `fix/mesh-session-cwd` / P0 | **First prerequisite; unfinished dirty work** | Snapshot status/diff; commit only in its own clean repair flow; independent review, targeted session-workspace tests, full required checks; land first. |
| #769 / `fix/reconcile-rmcp-sse-compat` | **Blocked replacement; automerge disabled** | Finish checks and independent review; wait for P0 merge/post-check; rebase if needed; enable squash automerge only after all gates. |
| #759 | **Blocked Cargo dependency batch** | Diagnose failures; retain unique updates; repair/rebase or supersede with reviewed replacements. Serialize with every other lockfile PR. |
| #739, #738, #737 | **Blocked Cargo dependency updates** | Inspect advisory/deny failures and compatibility; process one lockfile-changing PR at a time after predecessors. |
| #735 | **Independent but not merge-ready** | Review current diff/checks; repair if needed; independent approval, green required checks, then squash. |
| `fix/mesh-postcheck-scope` | **Dirty active follow-up** | Preserve; compare with P0 and main after P0; land separately only if unique and needed. |
| `fix/oauth-headless`, `handoff/oauth-wip`, root OAuth WIP | **Unique unfinished OAuth work** | Reconcile all committed/uncommitted variants; preserve PKCE/state protections; create a clean reviewed PR only after diff ownership is resolved. |
| `handoff/autocomplete`, `handoff/eas`, `handoff/header`, `handoff/liveactivity` | **Unique handoff work; no PR yet** | Rebase/verify from current main in separate worktrees; native/EAS checks where required; independently review and open PRs. User must provide `EXPO_TOKEN` for EAS. |
| Merged branches/PR heads (including #724, #730, #731, #741–#761 and other merged history) | **Candidate cleanup only** | Prove all unique commits and working-tree changes are integrated or superseded before archive/delete. |
| Closed/unmerged PR heads and frozen/overhaul branches | **Unresolved until compared** | Preserve unique work; classify as active, superseded, obsolete, or archiveable with evidence. |
| Agent/session worktrees | **Unknown ownership by default** | Inspect owner, status, head, unique commits, and PR; archive only after proof. |

## Ordered execution and gates

1. **Snapshot before each operation.** Refresh remote refs if needed (`git fetch origin --prune`, no branch mutation), then capture status, worktrees, branches, PR state, reviews, mergeability, protections, and checks.
2. **Preserve and map work.** For every worktree and branch run status/diff checks and compare `git log origin/main..REF`, `git diff origin/main...REF`, and patch-equivalence checks. Export or commit unique work only in its owner-approved worktree; never reset dirty WIP.
3. **P0 first.** Isolate `fix/mesh-session-cwd` from its dirty worktree into a clean branch based on current `origin/main`, retaining every unique file and test. Run targeted workspace-isolation/session-cwd tests, fmt, clippy, locked tests/build/security checks, then obtain independent review. Do not advance other dependent landings until P0 is merged and post-merge checks pass.
4. **Reconcile #769.** Rebase the rmcp compatibility repair onto post-P0 main if required, validate that it contains only intended dependency work, obtain independent review, wait for green current-head checks, then enable squash automerge. Keep disabled otherwise.
5. **Serialize dependency PRs.** For #759/#739/#738/#737 and any replacement, merge at most one root `Cargo.lock` changer at a time. After each squash merge: refresh main, rebase the next candidate, regenerate/verify lockfile, rerun fmt/clippy/tests/release/audit/deny, and repeat review for changed heads. Apply the same rule to mobile/package-lock changes.
6. **Process independent work.** Review/repair #735 and each handoff/OAuth/mesh follow-up from clean current-main worktrees. Run applicable Rust, security, mobile/native, E2E, and EAS checks; require independent review before squash automerge.
7. **Final proof and cleanup.** Re-inventory PRs, refs, worktrees, unique commits, and dirty files. For every cleanup candidate record `git status --short`, `git diff`, `git diff --cached`, head SHA, path, PR, and `git log origin/main..REF`; delete/archive only proven inactive merged/superseded items. Never touch the allowlist or unique unfinished work. Final state should contain `main` and genuinely active work only.

## Safe command set

```sh
# Inventory only
git status --short
git worktree list --porcelain
git branch -vv --all
git fetch origin --prune
git log --all --decorate --oneline
git log origin/main..REF --oneline
git diff --stat origin/main...REF
git diff --no-ext-diff origin/main...REF
gh pr list --state open --limit 100 --json number,title,headRefName,baseRefName,state,mergeable,reviewDecision,statusCheckRollup,url
gh pr view N --json commits,reviews,reviewDecision,mergeStateStatus,autoMergeRequest,statusCheckRollup,url

# Clean repair (only a newly created, non-owned worktree)
git worktree add /tmp/repair-<name> -b repair/<name> origin/main
# transfer only explicitly preserved unique work; validate, commit, push, open/update PR

# Gate and merge (only after independent review + green current checks)
gh pr checks N
gh pr review N --approve
gh pr merge N --squash --auto

# Cleanup only after recorded proof
git worktree remove /path/to/proven-inactive-worktree
git branch -d proven/inactive-branch
git push origin --delete proven/inactive-branch
```

Do not run cleanup commands against the root `pr/oauth`, any allowlisted path, or any item whose unique work/ownership proof is incomplete.

## Final verification checklist

- P0 is merged first and post-merge checks are green.
- #769 remained automerge-disabled until that point and was independently reviewed before squash.
- Every repaired PR has independent review, current required checks, and a recorded merge SHA.
- Every Cargo.lock/package-lock conflict was serialized and revalidated after predecessor merges.
- Every unique committed and uncommitted change has a landed, active, or explicit preserved disposition.
- Root `pr/oauth` WIP is unchanged.
- No inactive branch/session was removed without proof; only `main` and genuinely active work remain.
- This document contains concise evidence and actions, not raw logs or generated metadata.
