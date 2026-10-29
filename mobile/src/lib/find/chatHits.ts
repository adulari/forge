import type { HistoryRow } from "../api";
import { textMatches } from "./matching";

export function isSearchableRow(row: HistoryRow): boolean {
  return row.role !== "tool" && row.role !== "system" && row.kind !== "tool" && row.kind !== "nudge";
}

/** Hit seqs from the loaded rows and the host's answer, each re-checked against what the row
 * shows (an older host ignores `q` and returns an ordinary page), oldest first. */
export function mergeHits(
  loaded: readonly HistoryRow[],
  remote: readonly HistoryRow[],
  query: string,
  caseSensitive: boolean,
  textOf: (row: HistoryRow) => string,
): number[] {
  const seqs = new Set<number>();
  for (const row of [...loaded, ...remote]) {
    if (isSearchableRow(row) && textMatches(textOf(row), query, caseSensitive)) seqs.add(row.seq);
  }
  return [...seqs].sort((a, b) => a - b);
}
