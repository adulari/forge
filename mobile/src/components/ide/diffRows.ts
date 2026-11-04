// Flat rows for the virtualized diff. Hunks, unified lines and split pairs all become one list
// of fixed-height rows, so opening a 5,000-line diff is a list of numbers, not 5,000 mounted views.
import type { GitDiffHunk } from "../../lib/api";
import { toSplitRows, toUnifiedRows, type DiffCell, type SplitRow, type UnifiedRow } from "../git/diffModel";

export type DiffMode = "unified" | "split";

export type DiffRow =
  | { kind: "hunk"; key: string; header: string; hunk: number }
  | { kind: "line"; key: string; cell: DiffCell }
  | { kind: "pair"; key: string; left: DiffCell | null; right: DiffCell | null };

export const DIFF_LINE_HEIGHT = 20;
export const HUNK_HEIGHT = 28;

export interface DiffRows {
  rows: DiffRow[];
  /** Top offset of each row, for the list's fixed-height layout. */
  offsets: number[];
  /** Row index of each hunk header, in order: what n/p walk. */
  hunkStarts: number[];
}

export function buildDiffRows(hunks: readonly GitDiffHunk[], mode: DiffMode): DiffRows {
  const source: (UnifiedRow | SplitRow)[] = mode === "split" ? toSplitRows(hunks) : toUnifiedRows(hunks);
  const rows: DiffRow[] = [];
  const hunkStarts: number[] = [];
  for (const row of source) {
    if (row.kind === "hunk") {
      hunkStarts.push(rows.length);
      rows.push({ kind: "hunk", key: row.key, header: row.header, hunk: hunkStarts.length - 1 });
    } else if (row.kind === "line") {
      rows.push({ kind: "line", key: row.key, cell: row.cell });
    } else {
      rows.push({ kind: "pair", key: row.key, left: row.left, right: row.right });
    }
  }
  const offsets: number[] = new Array(rows.length);
  let top = 0;
  for (let i = 0; i < rows.length; i += 1) {
    offsets[i] = top;
    top += rows[i].kind === "hunk" ? HUNK_HEIGHT : DIFF_LINE_HEIGHT;
  }
  return { rows, offsets, hunkStarts };
}

/** The hunk to land on from the first visible row: the next start below it (or above, going back). */
export function hunkTarget(hunkStarts: readonly number[], fromRow: number, step: 1 | -1): number | null {
  if (hunkStarts.length === 0) return null;
  if (step === 1) return hunkStarts.find((start) => start > fromRow) ?? null;
  for (let i = hunkStarts.length - 1; i >= 0; i -= 1) if (hunkStarts[i] < fromRow) return hunkStarts[i];
  return null;
}
