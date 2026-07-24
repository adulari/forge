// Pure hunk → row derivation for the git review dock (docs/design/machined
// "Forge Machined - Desktop.dc.html" L265-308, D Git Review).
//
// Lives outside the components because the split view's pairing rule — which `-` line lines
// up against which `+` line — is the one piece of real logic in the dock, and it must produce
// the SAME line numbers as the unified view from the same hunk. Both modes derive from here.
//
// Input is `GitDiffHunk` verbatim from the daemon: each line keeps its gutter character as its
// first byte, and the daemon already drops `\ No newline at end of file` markers, so a line's
// first byte is only ever `+`, `-`, or a space.
import { type GitDiffHunk } from "../../lib/api";

export type LineKind = "context" | "add" | "del";

export interface DiffCell {
  /** Line number on this cell's own side of the diff. */
  lineNo: number;
  /** The line with its gutter character removed — the gutter is expressed as `kind`. */
  text: string;
  kind: LineKind;
}

/** One rendered row of the split view. A side is null where the change is unpaired (a pure
 * addition has no old-side line, a pure deletion has no new-side line) — the renderer paints
 * that side as an empty gutter-tinted filler, never as a fabricated blank line of code. */
export type SplitRow =
  | { kind: "hunk"; key: string; header: string }
  | { kind: "pair"; key: string; left: DiffCell | null; right: DiffCell | null };

export type UnifiedRow =
  | { kind: "hunk"; key: string; header: string }
  | { kind: "line"; key: string; cell: DiffCell };

interface HunkStart {
  oldStart: number;
  newStart: number;
}

const HUNK_HEADER = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/** `@@ -62,6 +62,7 @@ def sweep(...)` → the two 1-based start lines. A header the daemon
 * passed through unparsed (never seen in practice) falls back to 1/1 rather than throwing —
 * the diff body is still worth showing. */
export function parseHunkStart(header: string): HunkStart {
  const match = HUNK_HEADER.exec(header);
  if (!match) return { oldStart: 1, newStart: 1 };
  return { oldStart: Number(match[1]), newStart: Number(match[2]) };
}

function gutterOf(line: string): "+" | "-" | " " {
  const first = line[0];
  return first === "+" || first === "-" ? first : " ";
}

export function toSplitRows(hunks: readonly GitDiffHunk[]): SplitRow[] {
  const rows: SplitRow[] = [];
  hunks.forEach((hunk, hunkIndex) => {
    const { oldStart, newStart } = parseHunkStart(hunk.header);
    rows.push({ kind: "hunk", key: `h${hunkIndex}`, header: hunk.header });
    let oldNo = oldStart;
    let newNo = newStart;
    let dels: DiffCell[] = [];
    let adds: DiffCell[] = [];
    let pairIndex = 0;

    // A run of consecutive -/+ lines is one edit block: pair them off index-wise so a
    // replaced line sits opposite its replacement, and let the longer side spill into
    // one-sided rows.
    const flush = () => {
      const height = Math.max(dels.length, adds.length);
      for (let i = 0; i < height; i += 1) {
        rows.push({
          kind: "pair",
          key: `${hunkIndex}:p${pairIndex}`,
          left: dels[i] ?? null,
          right: adds[i] ?? null,
        });
        pairIndex += 1;
      }
      dels = [];
      adds = [];
    };

    for (const line of hunk.lines) {
      const gutter = gutterOf(line);
      const text = line.slice(1);
      if (gutter === "-") {
        dels.push({ lineNo: oldNo, text, kind: "del" });
        oldNo += 1;
        continue;
      }
      if (gutter === "+") {
        adds.push({ lineNo: newNo, text, kind: "add" });
        newNo += 1;
        continue;
      }
      flush();
      rows.push({
        kind: "pair",
        key: `${hunkIndex}:p${pairIndex}`,
        left: { lineNo: oldNo, text, kind: "context" },
        right: { lineNo: newNo, text, kind: "context" },
      });
      pairIndex += 1;
      oldNo += 1;
      newNo += 1;
    }
    flush();
  });
  return rows;
}

export function toUnifiedRows(hunks: readonly GitDiffHunk[]): UnifiedRow[] {
  const rows: UnifiedRow[] = [];
  hunks.forEach((hunk, hunkIndex) => {
    const { oldStart, newStart } = parseHunkStart(hunk.header);
    rows.push({ kind: "hunk", key: `h${hunkIndex}`, header: hunk.header });
    let oldNo = oldStart;
    let newNo = newStart;
    hunk.lines.forEach((line, lineIndex) => {
      const gutter = gutterOf(line);
      const text = line.slice(1);
      const key = `${hunkIndex}:l${lineIndex}`;
      if (gutter === "-") {
        rows.push({ kind: "line", key, cell: { lineNo: oldNo, text, kind: "del" } });
        oldNo += 1;
        return;
      }
      if (gutter === "+") {
        rows.push({ kind: "line", key, cell: { lineNo: newNo, text, kind: "add" } });
        newNo += 1;
        return;
      }
      rows.push({ kind: "line", key, cell: { lineNo: newNo, text, kind: "context" } });
      oldNo += 1;
      newNo += 1;
    });
  });
  return rows;
}

/** Mono middle-truncation. `ellipsizeMode="middle"` is native-only — react-native-web maps
 * every mode to CSS `text-overflow: ellipsis` (tail), and this dock's paths are only
 * distinguishable by their tail (`.../vol_mom.py`), so the budget is computed in characters
 * from the measured column width instead. */
export function middleTruncate(value: string, maxChars: number): string {
  if (maxChars <= 1 || value.length <= maxChars) return value;
  // Tail-biased: when the budget is odd the extra character goes to the filename end, which is
  // what distinguishes two rows in the list.
  const head = Math.floor((maxChars - 1) / 2);
  const tail = maxChars - 1 - head;
  return `${value.slice(0, head)}…${tail > 0 ? value.slice(value.length - tail) : ""}`;
}
