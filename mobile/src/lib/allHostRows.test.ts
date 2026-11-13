import { describe, expect, it } from "vitest";

import { allHostRows, waitingScoped, watchedServers } from "./allHostRows";
import { detectFleetAlerts, fleetRowKey } from "./notifyPrefsCore";

const row = (id: string, waiting: boolean, busy = false) => ({ id, title: id, waiting, busy, last_activity: 1 });

describe("allHostRows", () => {
  it("counts a session waiting on a non-active host", () => {
    const rows = allHostRows(
      [{ id: "a" }, { id: "b" }],
      "a",
      [row("s1", false)],
      new Map([["b", [row("s2", true)]]]),
    );
    const waiting = waitingScoped(rows);
    expect(waiting.map((item) => [item.serverId, item.row.id])).toEqual([["b", "s2"]]);
  });

  it("does not double count a daemon paired twice and prefers the active host", () => {
    const rows = allHostRows([{ id: "a" }, { id: "b" }], "a", [row("s1", true)], new Map([["b", [row("s1", true)]]]));
    expect(waitingScoped(rows).map((item) => item.serverId)).toEqual(["a"]);
  });

  it("tolerates a host that has not answered yet", () => {
    expect(allHostRows([{ id: "a" }, { id: "b" }], "a", undefined, new Map())).toEqual([]);
  });
});

describe("watchedServers", () => {
  it("skips disabled hosts except the active one", () => {
    const servers = [{ id: "a", disabled: true }, { id: "b", disabled: true }, { id: "c" }];
    expect(watchedServers(servers, "a").map((s) => s.id)).toEqual(["a", "c"]);
  });
});

describe("cross-host alerts", () => {
  it("raises attention for another host and carries its server id", () => {
    const before = { id: "s2", title: "t", busy: true, waiting: false, serverId: "b" };
    const after = { ...before, waiting: true };
    const alerts = detectFleetAlerts(new Map([[fleetRowKey(before), before]]), [after]);
    expect(alerts).toEqual([{ serverId: "b", sessionId: "s2", title: "t", category: "attention" }]);
  });

  it("does not confuse equal session ids on different hosts", () => {
    const a = { id: "s", title: "t", busy: false, waiting: false, serverId: "a" };
    const b = { ...a, serverId: "b", waiting: true };
    expect(detectFleetAlerts(new Map([[fleetRowKey(a), a]]), [b])).toEqual([]);
  });
});
