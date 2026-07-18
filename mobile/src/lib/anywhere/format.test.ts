import { describe, expect, it } from "vitest";

import { entitlementBadge, formatBytes, hostStateText, syncGlyph } from "./format";
import type { AnywhereHost, SyncStatus } from "./types";

describe("formatBytes", () => {
  it("formats sub-KB as bytes", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("formats KB", () => {
    expect(formatBytes(4096)).toBe("4 KB");
  });

  it("formats MB with one decimal", () => {
    expect(formatBytes(Math.round(18.2 * 1024 ** 2))).toBe("18.2 MB");
  });

  it("formats GB with one decimal", () => {
    expect(formatBytes(Math.round(1.2 * 1024 ** 3))).toBe("1.2 GB");
  });
});

describe("entitlementBadge", () => {
  it("renders trial with days remaining, warn tone", () => {
    expect(entitlementBadge({ entitlement: "trial", trialDaysLeft: 9 })).toEqual({
      label: "TRIAL · 9D",
      tone: "warn",
    });
  });

  it("renders active, success tone", () => {
    expect(entitlementBadge({ entitlement: "active" })).toEqual({ label: "ACTIVE", tone: "success" });
  });

  it("renders grace with days remaining, warn tone", () => {
    expect(entitlementBadge({ entitlement: "grace", graceDaysLeft: 6 })).toEqual({
      label: "GRACE · 6D",
      tone: "warn",
    });
  });

  it("renders read-only, danger tone", () => {
    expect(entitlementBadge({ entitlement: "read-only" })).toEqual({ label: "READ-ONLY", tone: "danger" });
  });

  it("renders suspended without a deadline as bare SUSPENDED", () => {
    expect(entitlementBadge({ entitlement: "suspended" })).toEqual({ label: "SUSPENDED", tone: "danger" });
  });

  it("renders suspended with a deadline as DELETES IN Nd", () => {
    expect(entitlementBadge({ entitlement: "suspended", deletesInDays: 12 })).toEqual({
      label: "DELETES IN 12D",
      tone: "danger",
    });
  });

  it("renders not-started, outline tone", () => {
    expect(entitlementBadge({ entitlement: "not-started" })).toEqual({ label: "NOT STARTED", tone: "outline" });
  });
});

describe("hostStateText", () => {
  const now = Date.parse("2026-07-18T12:00:00Z");

  it("renders online idle", () => {
    const host: Pick<AnywhereHost, "state"> = { state: { kind: "online", activity: "idle" } };
    expect(hostStateText(host, now)).toBe("online · idle");
  });

  it("renders online busy with session count", () => {
    const host: Pick<AnywhereHost, "state"> = { state: { kind: "online", activity: "busy", sessionCount: 2 } };
    expect(hostStateText(host, now)).toBe("online · busy · 2 sessions");
  });

  it("renders stale with relative last-seen", () => {
    const host: Pick<AnywhereHost, "state"> = { state: { kind: "stale", lastSeenAt: now - 26 * 60_000 } };
    expect(hostStateText(host, now)).toBe("stale · last seen 26m");
  });

  it("renders revoked", () => {
    const host: Pick<AnywhereHost, "state"> = { state: { kind: "revoked" } };
    expect(hostStateText(host, now)).toBe("revoked");
  });

  it("renders update-required with the connector version", () => {
    const host: Pick<AnywhereHost, "state"> = { state: { kind: "update-required", connectorVersion: "0.38.2" } };
    expect(hostStateText(host, now)).toBe("update required · v0.38.2");
  });
});

describe("syncGlyph", () => {
  const now = Date.parse("2026-07-18T12:00:00Z");

  it("renders synced with the check glyph, success color", () => {
    const status: SyncStatus = { kind: "synced", syncedAt: now - 2 * 60_000, offlineAvailable: true };
    expect(syncGlyph(status, now)).toEqual({
      glyph: "✓",
      colorKey: "success",
      text: "current · synced 2m ago · available offline",
    });
  });

  it("renders over-quota with the block glyph, danger color", () => {
    const status: SyncStatus = { kind: "over-quota" };
    expect(syncGlyph(status, now)).toEqual({
      glyph: "■",
      colorKey: "danger",
      text: "over quota — writes blocked · download/delete ok",
    });
  });

  it("renders retrying with attempt count, warn color", () => {
    const status: SyncStatus = { kind: "retrying", attempt: 3, lastSuccessAt: null };
    expect(syncGlyph(status, now)).toEqual({
      glyph: "↻",
      colorKey: "warn",
      text: "retrying · attempt 3",
    });
  });
});
