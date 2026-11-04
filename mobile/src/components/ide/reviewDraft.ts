// The review draft as plain functions: comments pile up per agent, then go out as one prompt
// ("Send review") or one at a time. Built on lib/reviewComments (prompt formatting, selection).
import { formatReviewCommentsPrompt, type ReviewComment } from "../../lib/reviewComments";

export function draftLabel(count: number): string {
  return count === 1 ? "1 comment" : `${count} comments`;
}

/** One prompt carrying every comment, in the order they were written. */
export function buildReviewPrompt(comments: readonly ReviewComment[]): string {
  return formatReviewCommentsPrompt("Please address this review.", comments);
}

/** A single comment sent on its own. */
export function buildSingleCommentPrompt(comment: ReviewComment): string {
  return formatReviewCommentsPrompt("Please address this review comment.", [comment]);
}

export interface FileDraftGroup {
  path: string;
  comments: ReviewComment[];
}

export function groupDraftByFile(comments: readonly ReviewComment[]): FileDraftGroup[] {
  const order: string[] = [];
  const byPath = new Map<string, ReviewComment[]>();
  for (const comment of comments) {
    if (!byPath.has(comment.path)) {
      byPath.set(comment.path, []);
      order.push(comment.path);
    }
    byPath.get(comment.path)?.push(comment);
  }
  return order.map((path) => ({ path, comments: byPath.get(path) ?? [] }));
}

const EXPLAIN_MAX_LINES = 60;

/** The /btw side question for "Explain this change": answered without interrupting the turn. */
export function explainPrompt(path: string, header: string, lines: readonly string[]): string {
  const shown = lines.slice(0, EXPLAIN_MAX_LINES);
  const cut = lines.length > shown.length ? `\n... ${lines.length - shown.length} more lines` : "";
  return `/btw Explain this change in ${path} (${header.trim()}). Why was it made and what does it affect?\n\n${shown.join("\n")}${cut}`;
}
