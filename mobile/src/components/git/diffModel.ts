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

export interface DiffTextSegment {
  text: string;
  changed: boolean;
}

export interface DiffCell {
  /** Line number on this cell's own side of the diff. */
  lineNo: number;
  /** The line with its gutter character removed — the gutter is expressed as `kind`. */
  text: string;
  kind: LineKind;
  /** Intraline comparison against the paired replacement line. Omitted for context/unpaired rows. */
  segments?: DiffTextSegment[];
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

const MAX_WORD_DIFF_TOKENS = 160;

function textTokens(value: string): string[] {
  return value.match(/(\s+|[A-Za-z0-9_$]+|.)/g) ?? [];
}

function pushSegment(segments: DiffTextSegment[], text: string, changed: boolean): void {
  if (!text) return;
  const previous = segments[segments.length - 1];
  if (previous?.changed === changed) previous.text += text;
  else segments.push({ text, changed });
}

function boundedFallbackSegments(oldText: string, newText: string): {
  oldSegments: DiffTextSegment[];
  newSegments: DiffTextSegment[];
} {
  let prefix = 0;
  const maxPrefix = Math.min(oldText.length, newText.length);
  while (prefix < maxPrefix && oldText[prefix] === newText[prefix]) prefix += 1;
  let suffix = 0;
  while (
    suffix < oldText.length - prefix &&
    suffix < newText.length - prefix &&
    oldText[oldText.length - 1 - suffix] === newText[newText.length - 1 - suffix]
  ) {
    suffix += 1;
  }
  const oldSegments: DiffTextSegment[] = [];
  const newSegments: DiffTextSegment[] = [];
  pushSegment(oldSegments, oldText.slice(0, prefix), false);
  pushSegment(newSegments, newText.slice(0, prefix), false);
  pushSegment(oldSegments, oldText.slice(prefix, oldText.length - suffix), true);
  pushSegment(newSegments, newText.slice(prefix, newText.length - suffix), true);
  if (suffix > 0) {
    pushSegment(oldSegments, oldText.slice(oldText.length - suffix), false);
    pushSegment(newSegments, newText.slice(newText.length - suffix), false);
  }
  return { oldSegments, newSegments };
}

/** Bounded token LCS. Code punctuation and whitespace remain tokens, which gives useful highlights
 * for identifiers, arguments, operators, and indentation without pulling a syntax engine into the
 * mobile bundle. Pathological generated lines fall back to a linear prefix/suffix comparison. */
export function diffTextSegments(oldText: string, newText: string): {
  oldSegments: DiffTextSegment[];
  newSegments: DiffTextSegment[];
} {
  if (oldText === newText) {
    return {
      oldSegments: [{ text: oldText, changed: false }],
      newSegments: [{ text: newText, changed: false }],
    };
  }
  const oldTokens = textTokens(oldText);
  const newTokens = textTokens(newText);
  if (oldTokens.length > MAX_WORD_DIFF_TOKENS || newTokens.length > MAX_WORD_DIFF_TOKENS) {
    return boundedFallbackSegments(oldText, newText);
  }

  const columns = newTokens.length + 1;
  const scores = new Uint16Array((oldTokens.length + 1) * columns);
  for (let oldIndex = oldTokens.length - 1; oldIndex >= 0; oldIndex -= 1) {
    for (let newIndex = newTokens.length - 1; newIndex >= 0; newIndex -= 1) {
      const offset = oldIndex * columns + newIndex;
      scores[offset] =
        oldTokens[oldIndex] === newTokens[newIndex]
          ? scores[(oldIndex + 1) * columns + newIndex + 1] + 1
          : Math.max(
              scores[(oldIndex + 1) * columns + newIndex],
              scores[oldIndex * columns + newIndex + 1],
            );
    }
  }

  const oldSegments: DiffTextSegment[] = [];
  const newSegments: DiffTextSegment[] = [];
  let oldIndex = 0;
  let newIndex = 0;
  while (oldIndex < oldTokens.length || newIndex < newTokens.length) {
    if (
      oldIndex < oldTokens.length &&
      newIndex < newTokens.length &&
      oldTokens[oldIndex] === newTokens[newIndex]
    ) {
      pushSegment(oldSegments, oldTokens[oldIndex], false);
      pushSegment(newSegments, newTokens[newIndex], false);
      oldIndex += 1;
      newIndex += 1;
    } else if (
      oldIndex < oldTokens.length &&
      (newIndex >= newTokens.length ||
        scores[(oldIndex + 1) * columns + newIndex] >=
          scores[oldIndex * columns + newIndex + 1])
    ) {
      pushSegment(oldSegments, oldTokens[oldIndex], true);
      oldIndex += 1;
    } else if (newIndex < newTokens.length) {
      pushSegment(newSegments, newTokens[newIndex], true);
      newIndex += 1;
    }
  }
  return { oldSegments, newSegments };
}

function annotateReplacementPair(
  deletion: DiffCell | undefined,
  addition: DiffCell | undefined,
): { deletion: DiffCell | null; addition: DiffCell | null } {
  if (!deletion || !addition) {
    return { deletion: deletion ?? null, addition: addition ?? null };
  }
  const { oldSegments, newSegments } = diffTextSegments(deletion.text, addition.text);
  return {
    deletion: { ...deletion, segments: oldSegments },
    addition: { ...addition, segments: newSegments },
  };
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
        const pair = annotateReplacementPair(dels[i], adds[i]);
        rows.push({
          kind: "pair",
          key: `${hunkIndex}:p${pairIndex}`,
          left: pair.deletion,
          right: pair.addition,
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
  const annotated: UnifiedRow[] = [];
  for (let index = 0; index < rows.length; ) {
    const row = rows[index];
    if (row.kind !== "line" || row.cell.kind !== "del") {
      annotated.push(row);
      index += 1;
      continue;
    }
    const deletions: Extract<UnifiedRow, { kind: "line" }>[] = [];
    const additions: Extract<UnifiedRow, { kind: "line" }>[] = [];
    while (index < rows.length) {
      const candidate = rows[index];
      if (candidate.kind !== "line" || candidate.cell.kind !== "del") break;
      deletions.push(candidate);
      index += 1;
    }
    while (index < rows.length) {
      const candidate = rows[index];
      if (candidate.kind !== "line" || candidate.cell.kind !== "add") break;
      additions.push(candidate);
      index += 1;
    }
    deletions.forEach((deletion, pairIndex) => {
      const pair = annotateReplacementPair(deletion.cell, additions[pairIndex]?.cell);
      annotated.push({ ...deletion, cell: pair.deletion ?? deletion.cell });
    });
    additions.forEach((addition, pairIndex) => {
      const pair = annotateReplacementPair(deletions[pairIndex]?.cell, addition.cell);
      annotated.push({ ...addition, cell: pair.addition ?? addition.cell });
    });
  }
  return annotated;
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
