// Undo for a reverted hunk or file. The daemon's `revert` action is `git apply -R` on the working
// tree, so applying it to the reversed patch re-applies the original: no new route is needed.
// Pure: nothing here touches the network or react-native.
import type { GitDiffFile, GitDiffHunk, GitHunkRequest } from "../../lib/api";
import { buildHunkPatch } from "../git/hunkActions";

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;

/** `@@ -a,b +c,d @@ ctx` → `@@ -c,d +a,b @@ ctx`; the optional counts are kept verbatim. */
export function reverseHunkHeader(header: string): string {
  const match = HUNK_RE.exec(header);
  if (!match) return header;
  const [, a, b, c, d, rest] = match;
  const old = b === undefined ? a : `${a},${b}`;
  const neu = d === undefined ? c : `${c},${d}`;
  return `@@ -${neu} +${old} @@${rest}`;
}

/** The hunk with additions and deletions swapped. Context and `\ No newline` lines are unchanged. */
export function reverseHunk(hunk: GitDiffHunk): GitDiffHunk {
  return {
    header: reverseHunkHeader(hunk.header),
    lines: hunk.lines.map((line) => {
      if (line.startsWith("+")) return `-${line.slice(1)}`;
      if (line.startsWith("-")) return `+${line.slice(1)}`;
      return line;
    }),
  };
}

export interface UndoStep {
  hunk_header: string;
  patch: string;
}

/** What to POST (`action: "revert"`) to put a reverted hunk back. */
export function undoStepForHunk(file: Pick<GitDiffFile, "path" | "kind">, hunk: GitDiffHunk): UndoStep {
  const reversed = reverseHunk(hunk);
  // The file header stays a/<path> b/<path>: the daemon only accepts that pair.
  return { hunk_header: reversed.header, patch: buildHunkPatch({ path: file.path, kind: "modified" }, reversed) };
}

/**
 * Steps that restore a whole reverted file, one per hunk, in file order. Empty when the file
 * cannot be rebuilt from its hunks: a cut-short diff, binary, rename, or a file that was created
 * or deleted (those need the file itself, not a patch of lines).
 */
export function undoStepsForFile(file: GitDiffFile): UndoStep[] {
  if (file.binary || file.skipped_lines > 0 || file.kind !== "modified" || file.orig_path) return [];
  return file.hunks.map((hunk) => undoStepForHunk(file, hunk));
}

export function undoRequests(session: string, path: string, steps: readonly UndoStep[]): GitHunkRequest[] {
  return steps.map((step) => ({ session, path, hunk_header: step.hunk_header, patch: step.patch, action: "revert" }));
}
