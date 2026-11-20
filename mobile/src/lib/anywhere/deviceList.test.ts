import { describe, expect, it } from "vitest";

import type { AnywhereDevice } from "../anywhereApi";
import { canMakeOwner, describeDevices, deviceKind, formatAgo } from "./deviceList";

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

describe("who may revoke whom", () => {
  const owned = (id: string, owner: boolean) => ({ ...device(id, `Device ${id}`, 10, 1), owner });

  it("lets any other device revoke a peer but only the owner revoke the owner", () => {
    const fromPeer = describeDevices([owned("o", true), owned("p", false), owned("q", false)], "p", NOW);
    const byId = Object.fromEntries(fromPeer.map((row) => [row.id, row]));
    expect(byId.o.isOwner).toBe(true);
    expect(byId.o.canRevoke).toBe(false);
    expect(byId.q.canRevoke).toBe(true);
    expect(byId.p.canRevoke).toBe(false);

    const fromOwner = describeDevices([owned("o", true), owned("p", false)], "o", NOW);
    expect(fromOwner.find((row) => row.id === "p")?.canRevoke).toBe(true);
    expect(fromOwner.find((row) => row.id === "o")?.canRevoke).toBe(false);
  });

  it("leaves the decision to an older service that reports no owner", () => {
    const rows = describeDevices([device("a", "A", 10, 1), device("b", "B", 10, 1)], "b", NOW);
    expect(rows.find((row) => row.id === "a")).toMatchObject({ isOwner: false, canRevoke: true });
  });
});

describe("canMakeOwner", () => {
  const other = { isOwner: false, isThisDevice: false };
  const on = { thisDeviceIsOwner: true, ownerTransferSupported: true };
  it("never without the capability", () => {
    expect(canMakeOwner(other, { ...on, ownerTransferSupported: undefined })).toBe(false);
    expect(canMakeOwner(other, { ...on, ownerTransferSupported: false })).toBe(false);
  });
  it("never when this device is not the owner", () => {
    expect(canMakeOwner(other, { ...on, thisDeviceIsOwner: false })).toBe(false);
  });
  it("never for the owner row or this device", () => {
    expect(canMakeOwner({ isOwner: true, isThisDevice: false }, on)).toBe(false);
    expect(canMakeOwner({ isOwner: false, isThisDevice: true }, on)).toBe(false);
  });
  it("yes for another device when owner and supported", () => {
    expect(canMakeOwner(other, on)).toBe(true);
  });
});
