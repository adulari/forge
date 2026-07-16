import { describe, expect, it } from "vitest";

import type { SessionRow } from "./api";
import { buildFleetDeck } from "./fleetRows";

function row(index: number, state: "waiting" | "busy" | "idle"): SessionRow {
  return {
    id: `s${index}`,
    title: `session ${index}`,
    cwd: "/tmp",
    worktree: null,
    busy: state === "busy",
    waiting: state === "waiting",
    cost_usd: 0,
    context_tokens: 0,
    context_limit: null,
    model: "mock",
    created_at: index,
    last_activity: index,
  };
}

describe("fleet deck", () => {
  it("groups rows and carries stable source indices without render-time searches", () => {
    const all = [row(0, "waiting"), row(1, "busy"), row(2, "idle")];
    expect(buildFleetDeck(all, all)).toEqual([
      { type: "label", label: "NEEDS YOU" },
      { type: "session", row: all[0], sourceIndex: 0 },
      { type: "label", label: "FORGING" },
      { type: "session", row: all[1], sourceIndex: 1 },
      { type: "label", label: "COOL" },
      { type: "session", row: all[2], sourceIndex: 2 },
    ]);
  });

  it("handles a thousand rows in one stable pass", () => {
    const all = Array.from({ length: 1_000 }, (_, index) => row(index, "idle"));
    const deck = buildFleetDeck(all, all);
    expect(deck).toHaveLength(1_001);
    expect(deck.at(-1)).toMatchObject({ type: "session", sourceIndex: 999 });
  });
});
