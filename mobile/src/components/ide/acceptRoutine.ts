// "Accept routine": the hunks the daemon ranked routine (lockfile churn, generated files,
// whitespace-only edits, import reorders) go through in one action. A shared tree stages them
// with the same hunk route the diff view uses; a worktree session has no index to stage into, so
// its fully routine files are marked reviewed, which is what `r` does. Pure: the planning and the
// requests are tested without a network.
import type { GitDiffFile, GitHunkRequest } from "../../lib/api";
import { buildHunkPatch } from "../git/hunkActions";
import type { QueueRow } from "./changeQueueModel";

export interface AcceptPlan {
  /** Shared-tree files with routine hunks left to stage. */
  stage: QueueRow[];
  /** Worktree files whose every hunk is routine. */
  review: QueueRow[];
}

/**
 * Which files an accept touches. A worktree file with a hunk that needs a look stays out: marking
 * it reviewed would hide that hunk. Staging needs the direct daemon routes, so a relayed
 * (read-only) connection only gets the review marks.
 */
export function planAcceptRoutine(rows: readonly QueueRow[], worktreeSessions: ReadonlySet<string>, readOnly: boolean): AcceptPlan {
  const plan: AcceptPlan = { stage: [], review: [] };
  for (const row of rows) {
    if (row.tier === "reviewed" || (row.file.routineHunks ?? 0) === 0) continue;
    if (worktreeSessions.has(row.sessionId)) {
      if ((row.file.lookHunks ?? 0) === 0) plan.review.push(row);
    } else if (!readOnly) {
      plan.stage.push(row);
    }
  }
  return plan;
}

/**
 * Stage requests for the routine hunks of one working-tree-vs-index diff, last hunk first so the
 * line numbers of the ones still to apply do not move. Nothing for a diff a patch cannot rebuild
 * faithfully (binary, rename, cut short).
 */
export function routineStageRequests(session: string, file: GitDiffFile): GitHunkRequest[] {
  if (file.binary || file.skipped_lines > 0 || file.kind === "renamed" || file.orig_path) return [];
  return file.hunks
    .filter((hunk) => hunk.risk === "routine")
    .map((hunk): GitHunkRequest => ({ session, path: file.path, hunk_header: hunk.header, patch: buildHunkPatch(file, hunk), action: "stage" }))
    .reverse();
}

/** The requests that take a staged accept back: the same hunks, unstaged in the order they were staged reversed. */
export function undoStageRequests(requests: readonly GitHunkRequest[]): GitHunkRequest[] {
  return [...requests].reverse().map((request) => ({ ...request, action: "unstage" }));
}

export interface AcceptSummary {
  hunks: number;
  files: number;
  failed: number;
}

/** "Accepted 31 routine hunks in 12 files", or what went wrong. */
export function acceptMessage({ hunks, files, failed }: AcceptSummary, reviewed: number): string {
  const parts: string[] = [];
  if (hunks > 0) parts.push(`staged ${hunks} routine ${hunks === 1 ? "hunk" : "hunks"} in ${files} ${files === 1 ? "file" : "files"}`);
  if (reviewed > 0) parts.push(`marked ${reviewed} routine ${reviewed === 1 ? "file" : "files"} reviewed`);
  const done = parts.length > 0 ? `${parts[0][0].toUpperCase()}${parts.join(", ").slice(1)}` : "Nothing routine to accept";
  return failed > 0 ? `${done}. ${failed} could not be applied (the file changed).` : done;
}
