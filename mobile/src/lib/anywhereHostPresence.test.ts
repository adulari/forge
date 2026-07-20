import { describe, expect, it } from "vitest";

import { hostFleetSummary, hostLastActiveMs, hostStatusText } from "./anywhereHostPresence";

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
});
