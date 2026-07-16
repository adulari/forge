import type { SessionRow } from "./api";

export type FleetDeckItem =
  | { type: "session"; row: SessionRow; sourceIndex: number }
  | { type: "label"; label: string };

/** Build section labels and preserve the source index in one linear pass. */
export function buildFleetDeck(
  filtered: readonly SessionRow[],
  all: readonly SessionRow[],
): FleetDeckItem[] {
  const sourceIndices = new Map(all.map((row, index) => [row.id, index]));
  const rows: FleetDeckItem[] = [];
  let previous: string | null = null;
  for (const row of filtered) {
    const group = row.waiting ? "waiting" : row.busy ? "busy" : "idle";
    if (group !== previous) {
      rows.push({
        type: "label",
        label: group === "waiting" ? "NEEDS YOU" : group === "busy" ? "FORGING" : "COOL",
      });
    }
    rows.push({ type: "session", row, sourceIndex: sourceIndices.get(row.id) ?? 0 });
    previous = group;
  }
  return rows;
}
