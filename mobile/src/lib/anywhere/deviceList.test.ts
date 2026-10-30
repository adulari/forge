import { describe, expect, it } from "vitest";

import type { AnywhereDevice } from "../anywhereApi";
import { describeDevices, deviceKind, formatAgo } from "./deviceList";

const NOW = Date.parse("2026-10-30T12:00:00Z");
const day = 24 * 60 * 60 * 1000;
const iso = (agoMs: number) => new Date(NOW - agoMs).toISOString();
function device(id: string, name: string, createdDays: number, seenDays: number | null): AnywhereDevice {
  return { id, name, created_at: iso(createdDays * day), last_seen_at: seenDays == null ? null : iso(seenDays * day), signing_public_key: "", exchange_public_key: "" };
}

describe("describeDevices", () => {
  const rows = describeDevices(
    [device("a", "Forge on iPhone", 40, null), device("b", "Forge on iPhone", 20, 2), device("c", "Forge Desktop", 5, 0), device("d", "Laptop", 90, 45)],
    "c",
    NOW,
  );

  it("puts this device first and orders the rest by recent activity", () => {
    expect(rows.map((row) => row.id)).toEqual(["c", "b", "a", "d"]);
    expect(rows[0].isThisDevice).toBe(true);
    expect(rows[0].lastSeen).toBe("Active now");
  });

  it("flags stale enrollments but never the current device", () => {
    expect(Object.fromEntries(rows.map((row) => [row.id, row.stale]))).toEqual({ c: false, b: false, a: true, d: true });
  });

  it("marks same-named enrollments so they can be told apart", () => {
    expect(rows.find((row) => row.id === "a")?.duplicateName).toBe(true);
    expect(rows.find((row) => row.id === "c")?.duplicateName).toBe(false);
  });

  it("describes never-seen and old devices honestly", () => {
    expect(rows.find((row) => row.id === "a")?.lastSeen).toBe("Never connected");
    expect(rows.find((row) => row.id === "b")?.lastSeen).toBe("Last seen 2d ago");
    expect(rows.find((row) => row.id === "a")?.enrolled).toMatch(/^Enrolled [A-Z][a-z]{2} \d+/);
  });
});

describe("helpers", () => {
  it("formats relative times", () => {
    expect(formatAgo(NOW - 30_000, NOW)).toBe("just now");
    expect(formatAgo(NOW - 3 * 60 * 60_000, NOW)).toBe("3h ago");
  });
  it("classifies device kind from the name", () => {
    expect(deviceKind("Forge on iPhone")).toBe("phone");
    expect(deviceKind("Forge Desktop")).toBe("computer");
  });
});
