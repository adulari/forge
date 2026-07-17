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
export function buildFleetDeck(
  filtered: readonly SessionRow[],
  all: readonly SessionRow[],
): FleetDeckItem[] {
  const sourceIndices = new Map(all.map((row, index) => [row.id, index]));
  const rows: FleetDeckItem[] = [];
  let previous: string | null = null;
  for (const row of filtered) {
    const group = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
    if (group !== previous && group !== "waiting") {
      rows.push({ type: "label", label: group === "busy" ? "FORGING" : "COOL" });
    }
    rows.push({ type: "session", row, sourceIndex: sourceIndices.get(row.id) ?? 0 });
    previous = group;
  }
  return rows;
}
