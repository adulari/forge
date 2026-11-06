import { describe, expect, it } from "vitest";

import {
  canReclaimFrom,
  createWarning,
  floorRefusal,
  formatBytes,
  isPressured,
  pressureMessage,
  reclaimLabel,
  reclaimResultMessage,
} from "./hostPressure";

describe("host pressure", () => {
  it("only warn and floor count as pressure", () => {
    expect(isPressured({ level: "ok" })).toBe(false);
    expect(isPressured(null)).toBe(false);
    expect(isPressured(undefined)).toBe(false);
    expect(isPressured({ level: "warn" })).toBe(true);
    expect(isPressured({ level: "floor" })).toBe(true);
  });

  it("says that new worktree sessions are paused under the floor, and only warns above it", () => {
    const summary = "projects disk: 2.0 GB free of 500.0 GB (0%)";
    expect(pressureMessage({ level: "floor", summary })).toBe(`New worktree sessions are paused. ${summary}`);
    expect(pressureMessage({ level: "warn", summary })).toBe(`Host running low. ${summary}`);
    expect(pressureMessage({ level: "warn" })).toContain("low on disk or memory");
  });

  it("formats byte counts the way the host does", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(30 * 1024 ** 3)).toBe("30.0 GB");
  });

  it("recognises the hard-floor refusal by its code and keeps the reclaim offer", () => {
    const refusal = floorRefusal({
      status: 507,
      message: "not creating a worktree: below the hard floor. Reclaim can free 30.0 GB",
      body: { code: "host_disk_floor", pressure: { level: "floor", summary: "x", reclaimable_bytes: 30 * 1024 ** 3 } },
    });
    expect(refusal?.gate.reclaimable_bytes).toBe(30 * 1024 ** 3);
    expect(refusal?.message).toContain("Reclaim can free");
    expect(reclaimLabel(refusal?.gate)).toBe("Reclaim 30.0 GB");
    expect(floorRefusal({ status: 400, message: "bad", body: { code: "other" } })).toBeNull();
    expect(floorRefusal(new Error("offline"))).toBeNull();
    expect(floorRefusal(null)).toBeNull();
  });

  it("warns after a low-disk create and stays silent when there is room", () => {
    expect(createWarning(null)).toBeNull();
    expect(createWarning({ level: "ok" })).toBeNull();
    expect(
      createWarning({ level: "warn", summary: "data disk: 9.0 GB free of 100.0 GB (9%)", reclaimable_bytes: 2 * 1024 ** 3 }),
    ).toBe("Session started, but data disk: 9.0 GB free of 100.0 GB (9%). 2.0 GB can be reclaimed.");
    expect(createWarning({ level: "warn", summary: "s", reclaimable_bytes: 0 })).toBe("Session started, but s.");
  });

  it("offers Reclaim only over a direct connection", () => {
    expect(canReclaimFrom("https://192.168.1.2:7420/tok")).toBe(true);
    expect(canReclaimFrom("fany://host-1")).toBe(false);
    expect(canReclaimFrom(null)).toBe(false);
  });

  it("summarises what a reclaim freed", () => {
    expect(reclaimResultMessage({ freed_bytes: 3 * 1024 ** 3, removed: 2, pruned: 1, failures: [] })).toBe(
      "Reclaimed 3.0 GB: 2 worktrees removed, 1 build dir pruned.",
    );
    expect(reclaimResultMessage({ freed_bytes: 0, removed: 0, pruned: 0, failures: ["a"] })).toContain("1 failed");
  });
});
