import type { DrainItemStatus, IssueDrain, IssueDrainRequest, IssueRow, IssueStartRequest } from "../../lib/apiIssues";
import type { PrWatchMethod, PrWatchMode } from "../../lib/apiPr";

export interface IssueEntry extends IssueRow {
  /** A session in the issue's project; the daemon resolves the repository through it. */
  sessionId: string;
}

/** Merge per-project results into one list: newest activity first, an issue seen twice (two
 * sessions in the same repository) kept once. */
export function mergeIssues(
  results: readonly ({ issues: IssueRow[] } | undefined)[],
  sessionIds: readonly string[],
): IssueEntry[] {
  const seen = new Set<string>();
  const out: IssueEntry[] = [];
  results.forEach((result, i) => {
    for (const issue of result?.issues ?? []) {
      if (seen.has(issue.url)) continue;
      seen.add(issue.url);
      out.push({ ...issue, sessionId: sessionIds[i] });
    }
  });
  return out.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
}

export function issueSubtitle(issue: IssueRow, ago: string): string {
  const comments = issue.comment_count === 1 ? "1 comment" : `${issue.comment_count} comments`;
  return [`#${issue.number}`, issue.author, ago, comments].filter((part) => part !== "").join(" - ");
}

/** The start request for the form's state. The merge method only matters when merging. */
export function startRequest(
  issue: IssueEntry,
  options: { worktree: boolean; mode: PrWatchMode; method: PrWatchMethod },
): IssueStartRequest {
  return {
    session: issue.sessionId,
    number: issue.number,
    worktree: options.worktree,
    mode: options.mode,
    ...(options.mode === "merge" ? { method: options.method } : {}),
  };
}

export function startHint(mode: PrWatchMode, worktree: boolean): string {
  const where = worktree ? "in a new worktree" : "in the project checkout";
  const watch =
    mode === "off"
      ? "PR Watch stays off."
      : mode === "fix"
        ? "PR Watch will fix failing checks and review comments."
        : "PR Watch will fix failing checks and merge when green.";
  return `Forge works ${where} and opens a pull request that closes the issue. ${watch}`;
}

export const MAX_DRAIN_PARALLEL = 8;
export const MAX_DRAIN_ISSUES = 30;

/** Spend envelopes the Drain sheet offers: a slice of the weekly Claude window. */
export const DRAIN_BUDGETS: { value: string; label: string }[] = [
  { value: "", label: "No limit" },
  { value: "claude:10%w", label: "10% weekly" },
  { value: "claude:20%w", label: "20% weekly" },
  { value: "claude:30%w", label: "30% weekly" },
  { value: "claude:50%w", label: "50% weekly" },
];

/** Toggle an issue in the selection. A drain works one repository, so picking from another repo
 * starts a new selection instead of mixing them. */
export function toggleSelection(selected: readonly IssueEntry[], issue: IssueEntry): IssueEntry[] {
  if (selected.some((s) => s.url === issue.url)) return selected.filter((s) => s.url !== issue.url);
  if (selected.length > 0 && selected[0].repo !== issue.repo) return [issue];
  return selected.length >= MAX_DRAIN_ISSUES ? [...selected] : [...selected, issue];
}

export function drainRequest(
  selected: readonly IssueEntry[],
  options: { parallel: number; budget: string; worktree: boolean; mode: PrWatchMode; method: PrWatchMethod },
): IssueDrainRequest {
  return {
    session: selected[0].sessionId,
    numbers: selected.map((s) => s.number),
    parallel: Math.min(Math.max(Math.round(options.parallel), 1), MAX_DRAIN_PARALLEL),
    worktree: options.worktree,
    mode: options.mode,
    ...(options.budget ? { budget: options.budget } : {}),
    ...(options.mode === "merge" ? { method: options.method } : {}),
  };
}

export function drainHint(count: number, parallel: number, budget: string): string {
  const spend = budget
    ? `It stops starting issues once ${DRAIN_BUDGETS.find((b) => b.value === budget)?.label ?? budget} is spent and carries on after the window resets.`
    : "There is no spend limit.";
  return `${count} ${count === 1 ? "issue" : "issues"}, ${parallel} at a time; the next starts when one opens its pull request or stops. ${spend}`;
}

export const DRAIN_ITEM_LABEL: Record<DrainItemStatus, string> = {
  queued: "queued",
  running: "running",
  waiting: "needs you",
  pr_open: "PR open",
  merged: "merged",
  done: "idle",
  failed: "failed",
  blocked: "blocked",
  cancelled: "cancelled",
};

/** The drain to show: the one still going, else the latest. */
export function pickDrain(drains: readonly IssueDrain[] | undefined): IssueDrain | null {
  if (!drains || drains.length === 0) return null;
  return drains.find((d) => d.status === "active" || d.status === "paused") ?? drains[0];
}

/** Issue number to its status in the drain, for tagging rows in the list. */
export function drainStatusByNumber(drain: IssueDrain | null): Map<number, DrainItemStatus> {
  return new Map((drain?.items ?? []).map((i) => [i.number, i.status]));
}

/** Whether a drain's project is the repository (`owner/name`) of an issue row: the daemon names
 * the drain's checkout, the issue row names the GitHub repository, so compare the folder name. */
export function drainCoversRepo(drain: IssueDrain | null, repo: string): boolean {
  if (!drain || !repo) return false;
  const name = repo.split("/").pop() ?? "";
  return name !== "" && drain.cwd.replace(/\/+$/, "").split("/").pop() === name;
}
