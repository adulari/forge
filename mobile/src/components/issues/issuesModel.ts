import type { IssueRow, IssueStartRequest } from "../../lib/apiIssues";
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
