import { describe, expect, it } from "vitest";

import { effectiveShowAll, mergeServerFleets, type MergeableRow } from "./fleetScope";

function row(id: string, state: "waiting" | "busy" | "idle", lastActivity: number): MergeableRow {
  return { id, waiting: state === "waiting", busy: state === "busy", last_activity: lastActivity };
}

const servers = [{ id: "a" }, { id: "b" }];

describe("mergeServerFleets", () => {
  it("keeps waiting, then busy, then idle across servers, most recent first", () => {
    const merged = mergeServerFleets(
      servers,
      [
        [row("a1", "waiting", 1), row("a2", "idle", 9)],
        [row("b1", "busy", 5), row("b2", "waiting", 7), row("b3", "idle", 10)],
      ],
      "a",
    );
    expect(merged.map((item) => `${item.serverId}:${item.row.id}`)).toEqual(["b:b2", "a:a1", "b:b1", "b:b3", "a:a2"]);
  });

  it("shows a session reachable through two servers once, from the preferred server", () => {
    const shared = row("s", "idle", 3);
    expect(mergeServerFleets(servers, [[shared], [shared]], "b")).toEqual([{ row: shared, serverId: "b" }]);
    expect(mergeServerFleets(servers, [[shared], [shared]], null)).toEqual([{ row: shared, serverId: "a" }]);
  });

  it("treats a server without data yet as empty", () => {
    expect(mergeServerFleets(servers, [undefined, [row("b1", "idle", 1)]], "a").map((item) => item.row.id)).toEqual(["b1"]);
  });
});

describe("effectiveShowAll", () => {
  it("only offers All with a second server", () => {
    expect(effectiveShowAll(true, 1)).toBe(false);
    expect(effectiveShowAll(true, 2)).toBe(true);
    expect(effectiveShowAll(false, 3)).toBe(false);
  });
});
