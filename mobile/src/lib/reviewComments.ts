import { useSyncExternalStore } from "react";

import { type LineKind } from "../components/git/diffModel";

export type ReviewCommentSide = "old" | "new";
export type ReviewCommentSource = "working-tree" | "turn" | "fork";

export interface ReviewCommentLine {
  lineNo: number;
  kind: LineKind;
  text: string;
}

export interface ReviewComment {
  id: string;
  sessionId: string;
  source: ReviewCommentSource;
  path: string;
  /** Hash of the rendered patch; prevents a pending marker from attaching to shifted later lines. */
  revision: string;
  staged: boolean;
  side: ReviewCommentSide;
  startLine: number;
  endLine: number;
  lines: ReviewCommentLine[];
  text: string;
}

export interface ReviewLineSelection {
  side: ReviewCommentSide;
  startLine: number;
  endLine: number;
  lines: ReviewCommentLine[];
}

const EMPTY_COMMENTS: readonly ReviewComment[] = [];
const commentsBySession = new Map<string, readonly ReviewComment[]>();
const listeners = new Set<() => void>();
let nextCommentSequence = 1;

function emit(): void {
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getReviewComments(sessionId: string): readonly ReviewComment[] {
  return commentsBySession.get(sessionId) ?? EMPTY_COMMENTS;
}

export function useReviewComments(sessionId: string): readonly ReviewComment[] {
  return useSyncExternalStore(
    subscribe,
    () => getReviewComments(sessionId),
    () => getReviewComments(sessionId),
  );
}

export function addReviewComment(
  comment: Omit<ReviewComment, "id" | "sessionId"> & { sessionId: string },
): ReviewComment {
  const created: ReviewComment = {
    ...comment,
    id: `review-${Date.now().toString(36)}-${nextCommentSequence.toString(36)}`,
  };
  nextCommentSequence += 1;
  commentsBySession.set(comment.sessionId, [...getReviewComments(comment.sessionId), created]);
  emit();
  return created;
}

export function removeReviewComment(sessionId: string, id: string): void {
  const current = getReviewComments(sessionId);
  const next = current.filter((comment) => comment.id !== id);
  if (next.length === current.length) return;
  if (next.length === 0) commentsBySession.delete(sessionId);
  else commentsBySession.set(sessionId, next);
  emit();
}

export function clearReviewComments(sessionId: string): void {
  if (!commentsBySession.delete(sessionId)) return;
  emit();
}

export function reviewRangeLabel(
  side: ReviewCommentSide,
  startLine: number,
  endLine: number,
): string {
  const prefix = side === "old" ? "old" : "new";
  return startLine === endLine
    ? `${prefix} L${startLine}`
    : `${prefix} L${startLine}–L${endLine}`;
}

export function buildReviewLineSelection(
  side: ReviewCommentSide,
  availableLines: readonly ReviewCommentLine[],
  anchorLine: number,
  targetLine: number,
): ReviewLineSelection {
  const startLine = Math.min(anchorLine, targetLine);
  const endLine = Math.max(anchorLine, targetLine);
  return {
    side,
    startLine,
    endLine,
    lines: availableLines
      .filter((line) => line.lineNo >= startLine && line.lineNo <= endLine)
      .sort((a, b) => a.lineNo - b.lineNo),
  };
}

export function reviewDiffRevision(
  path: string,
  hunks: readonly { header: string; lines: readonly string[] }[],
): string {
  let hash = 0x811c9dc5;
  const update = (value: string) => {
    for (let index = 0; index < value.length; index += 1) {
      hash ^= value.charCodeAt(index);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
  };
  update(path);
  hunks.forEach((hunk) => {
    update("\0");
    update(hunk.header);
    hunk.lines.forEach((line) => {
      update("\n");
      update(line);
    });
  });
  return hash.toString(16).padStart(8, "0");
}

export function formatReviewCommentsPrompt(
  prompt: string,
  comments: readonly ReviewComment[],
): string {
  if (comments.length === 0) return prompt.trim();
  const intro = prompt.trim() || "Please address the following review feedback.";
  const blocks = comments.map((comment, index) => {
    const range = reviewRangeLabel(comment.side, comment.startLine, comment.endLine);
    const bucket =
      comment.source === "turn"
        ? "turn diff"
        : comment.source === "fork"
          ? "fork diff"
          : comment.staged
            ? "staged"
            : "working tree";
    const path = comment.path.replaceAll("`", "\\`");
    const feedback = comment.text
      .trim()
      .split(/\r?\n/)
      .map((line) => `   ${line}`)
      .join("\n");
    const context = comment.lines
      .map((line) => {
        const gutter = line.kind === "add" ? "+" : line.kind === "del" ? "-" : " ";
        return `    ${line.lineNo.toString().padStart(4, " ")} ${gutter}${line.text}`;
      })
      .join("\n");
    return [
      `${index + 1}. \`${path}\` · ${range} · ${bucket}`,
      feedback,
      "   Context:",
      context || "    (no text context)",
    ].join("\n");
  });
  return `${intro}\n\nReview annotations:\n${blocks.join("\n\n")}`;
}

/** Test-only reset; intentionally not exported through any UI path. */
export function resetReviewCommentsForTests(): void {
  commentsBySession.clear();
  nextCommentSequence = 1;
  emit();
}
