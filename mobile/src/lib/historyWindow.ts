import type { HistoryRow } from "./api";

/** Whether a fetched window actually holds `seq` (a host that ignores `around` answers the
 * newest page, which only holds it when it was already loaded). */
export function windowHolds(rows: readonly HistoryRow[], seq: number): boolean {
  if (rows.length === 0) return false;
  return rows[rows.length - 1].seq <= seq && rows[0].seq >= seq;
}
