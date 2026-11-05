// Which lines a hunk-level comment or "Explain this change" is about, and which hunk the viewer is
// looking at. Pure, so the keyboard flow (a / x / c on "the current hunk") is tested.
import type { GitDiffHunk } from "../../lib/api";
import { parseHunkStart } from "../git/diffModel";
import type { ReviewCommentLine, ReviewLineSelection } from "../../lib/reviewComments";

/** The hunk whose header is at or above `firstRow`; the first hunk before any header scrolled by. */
export function currentHunkIndex(hunkStarts: readonly number[], firstRow: number): number {
  let current = 0;
  for (let i = 0; i < hunkStarts.length; i += 1) {
    if (hunkStarts[i] <= firstRow) current = i;
    else break;
  }
  return current;
}

/**
 * The span a comment on the whole hunk anchors to: the new-side range covering its additions, else
 * the old-side range of its deletions. Context between them is kept so the agent sees the shape.
 */
export function hunkSelection(hunk: GitDiffHunk): ReviewLineSelection | null {
  const start = parseHunkStart(hunk.header);
  let oldLine = start.oldStart;
  let newLine = start.newStart;
  const news: ReviewCommentLine[] = [];
  const olds: ReviewCommentLine[] = [];
  let firstAdd = -1;
  let lastAdd = -1;
  let firstDel = -1;
  let lastDel = -1;
  for (const line of hunk.lines) {
    const gutter = line[0];
    const text = line.slice(1);
    if (gutter === "\\") continue;
    if (gutter === "+") {
      if (firstAdd === -1) firstAdd = news.length;
      lastAdd = news.length;
      news.push({ lineNo: newLine, kind: "add", text });
      newLine += 1;
    } else if (gutter === "-") {
      if (firstDel === -1) firstDel = olds.length;
      lastDel = olds.length;
      olds.push({ lineNo: oldLine, kind: "del", text });
      oldLine += 1;
    } else {
      news.push({ lineNo: newLine, kind: "context", text });
      olds.push({ lineNo: oldLine, kind: "context", text });
      newLine += 1;
      oldLine += 1;
    }
  }
  if (firstAdd !== -1) {
    const lines = news.slice(firstAdd, lastAdd + 1);
    return { side: "new", startLine: lines[0].lineNo, endLine: lines[lines.length - 1].lineNo, lines };
  }
  if (firstDel !== -1) {
    const lines = olds.slice(firstDel, lastDel + 1);
    return { side: "old", startLine: lines[0].lineNo, endLine: lines[lines.length - 1].lineNo, lines };
  }
  return null;
}

/** One clicked line as a selection. */
export function lineSelection(cell: { lineNo: number; kind: "context" | "add" | "del"; text: string }): ReviewLineSelection {
  return {
    side: cell.kind === "del" ? "old" : "new",
    startLine: cell.lineNo,
    endLine: cell.lineNo,
    lines: [{ lineNo: cell.lineNo, kind: cell.kind, text: cell.text }],
  };
}
