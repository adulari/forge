import { describe, expect, it } from "vitest";

import { socketChipModel } from "./socketChip";

describe("socketChipModel", () => {
  it("names each state the socket can be in", () => {
    expect(socketChipModel("open", false)).toEqual({ label: "Live", tone: "ok" });
    expect(socketChipModel("connecting", false)?.label).toBe("Reconnecting");
    expect(socketChipModel("reconnecting", false)?.tone).toBe("warn");
    expect(socketChipModel("paused", false)?.label).toBe("Paused in background");
    expect(socketChipModel("unreachable", false)).toEqual({ label: "Offline", tone: "danger" });
  });

  it("says nothing before the first connect, and Ended once the session is gone", () => {
    expect(socketChipModel("idle", false)).toBeNull();
    expect(socketChipModel("reconnecting", true)?.label).toBe("Ended");
    expect(socketChipModel("closed", false)?.label).toBe("Ended");
  });
});
