// The cards of the stacked diff, derived from `GET /api/git/status`. One card per (bucket, path):
// a file edited after staging legitimately has a STAGED card and an UNSTAGED card, each diffing a
// different pair of trees. Conflicted paths get no card - git has no two-sided diff for them, and
// the file list offers them a resolver instead.
import { type GitFileRow, type GitStatusResponse } from "../../lib/api";

export type DiffBucket = "staged" | "unstaged" | "untracked";

export interface DiffEntry {
  key: string;
  path: string;
  staged: boolean;
  bucket: DiffBucket;
  row: GitFileRow;
}

/** Cards open on first paint: at most this many, and only while their counted lines stay small
 * enough that opening them is not a wall of text and a burst of requests. */
const EXPAND_MAX_CARDS = 6;
const EXPAND_MAX_LINES = 800;
/** "Expand all" stops here; every open card is a request to the daemon. */
export const EXPAND_ALL_LIMIT = 40;
/** Cards rendered before the "show more files" control. */
export const CARD_RENDER_LIMIT = 200;

export function diffEntryKey(bucket: DiffBucket, path: string): string {
  return `${bucket}:${path}`;
}

export function buildDiffEntries(status: GitStatusResponse): DiffEntry[] {
  const make = (bucket: DiffBucket, rows: GitFileRow[]): DiffEntry[] =>
    rows.map((row) => ({
      key: diffEntryKey(bucket, row.path),
      path: row.path,
      staged: bucket === "staged",
      bucket,
      row,
    }));
  return [
    ...make("staged", status.staged),
    ...make("unstaged", status.unstaged),
    ...make("untracked", status.untracked),
  ];
}

/** Keys of the cards that start open. Always the first card, then more while they stay small. */
export function defaultExpandedKeys(entries: readonly DiffEntry[]): Set<string> {
  const open = new Set<string>();
  let lines = 0;
  for (const entry of entries) {
    const size = entry.row.adds + entry.row.dels;
    if (open.size > 0 && (open.size >= EXPAND_MAX_CARDS || lines + size > EXPAND_MAX_LINES)) break;
    open.add(entry.key);
    lines += size;
  }
  return open;
}

/** The selection (path + bucket) the file list reports, as a card key. */
export function selectionKey(selection: { path: string; staged: boolean } | null): string | null {
  if (!selection) return null;
  return diffEntryKey(selection.staged ? "staged" : "unstaged", selection.path);
}
