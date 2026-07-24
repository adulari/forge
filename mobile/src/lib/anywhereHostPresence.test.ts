import { describe, expect, it } from "vitest";

import {
  hostFleetSummary,
  hostLastActiveMs,
  hostStatusText,
  managedHostPresence,
} from "./anywhereHostPresence";

describe("Anywhere host presence", () => {
  it("counts only live relay connections as online", () => {
    expect(hostFleetSummary([{ online: true }, { online: false }])).toBe("1 online · 2 hosts");
    expect(hostFleetSummary([{ online: true }])).toBe("1 online · 1 host");
  });

  it("labels current and disconnected hosts truthfully", () => {
    expect(hostStatusText({ online: true, last_heartbeat_at: null })).toBe("Online");
    expect(hostStatusText({ online: false, last_heartbeat_at: null })).toBe("Offline");
  });

  it("converts service Unix seconds to JavaScript milliseconds", () => {
    expect(hostLastActiveMs({ last_heartbeat_at: "123" })).toBe(123_000);
    expect(hostLastActiveMs({ last_heartbeat_at: "not-a-timestamp" })).toBeNull();
  });

  it("uses authoritative relay presence and Unix heartbeats for managed host state", () => {
    const now = 200_000;
    expect(managedHostPresence({ online: true, last_heartbeat_at: null }, now)).toEqual({
      heartbeatAgeSec: 0,
      state: { kind: "online", activity: "idle" },
    });
    expect(managedHostPresence({ online: false, last_heartbeat_at: "190" }, now)).toEqual({
      heartbeatAgeSec: 10,
      state: { kind: "stale", lastSeenAt: 190_000 },
    });
    expect(managedHostPresence({ online: false, last_heartbeat_at: "1" }, now)).toEqual({
      heartbeatAgeSec: 199,
      state: { kind: "offline", lastHeartbeatAt: 1_000 },
    });
  });
});
