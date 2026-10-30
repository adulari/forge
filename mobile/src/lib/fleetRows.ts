import type { SessionRow } from "./api";

export type FleetDeckItem =
  | { type: "session"; row: SessionRow; sourceIndex: number }
  | { type: "label"; label: string };

/**
 * Build section labels and preserve the source index in one linear pass.
 *
 * Hearth: a waiting row renders as its own elevated decision card (core rule 2) — that's
 * self-evidently "needs you", so no "NEEDS YOU" section label is emitted above it (matches
 * the Fleet prototype, which has no such heading). "FORGING" / "COOL" labels still mark the
 * busy/idle groups below.
 */
export const DONE_LABEL_PREFIX = "DONE";

export function buildFleetDeck(
  filtered: readonly SessionRow[],
  all: readonly SessionRow[],
  showDone = false,
): FleetDeckItem[] {
  const sourceIndices = new Map(all.map((row, index) => [row.id, index]));
  const rows: FleetDeckItem[] = [];
  const done = filtered.filter((row) => row.done_at != null);
  const live = filtered.filter((row) => row.done_at == null);
  const groupOf = (row: SessionRow) =>
    row.waiting ? "waiting" : row.starred ? "starred" : row.busy ? "busy" : "idle";
  const labels = { starred: "STARRED", busy: "FORGING", idle: "COOL" } as const;
  const ordered = [...live].sort((a, b) => order(groupOf(a)) - order(groupOf(b)));
  let previous: string | null = null;
  for (const row of ordered) {
    const group = groupOf(row);
    if (group !== previous && group !== "waiting") {
      rows.push({ type: "label", label: labels[group] });
    }
    rows.push({ type: "session", row, sourceIndex: sourceIndices.get(row.id) ?? 0 });
    previous = group;
  }
  if (done.length > 0) {
    rows.push({ type: "label", label: `${DONE_LABEL_PREFIX} · ${done.length}` });
    if (showDone) {
      for (const row of done) {
        rows.push({ type: "session", row, sourceIndex: sourceIndices.get(row.id) ?? 0 });
      }
    }
  }
  return rows;
}

function order(group: string): number {
  return group === "waiting" ? 0 : group === "starred" ? 1 : group === "busy" ? 2 : 3;
}
