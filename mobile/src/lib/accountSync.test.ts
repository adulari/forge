import { describe, expect, it } from "vitest";

import {
  ageLabel,
  categoryStatusLine,
  classifyAccountSyncError,
  syncHubMeta,
  syncNeedsAttention,
  syncPillModel,
  toggleLockReason,
  withCategoryEnabled,
} from "./accountSync";
import type { AccountSyncCategory, AccountSyncStatus } from "./api";

const ApiError = class extends Error {
  name = "ApiError";
  constructor(public status: number, message: string) {
    super(message);
  }
};

const NOW = 1_800_000_000_000;
const category = (patch: Partial<AccountSyncCategory> = {}): AccountSyncCategory => ({
  id: "skills", label: "Skills & agents", enabled: false, items: 0, pending: 0, last_sync_at: null,
  remote_only: null, conflicts: [], skipped: [], toggle_requires_direct: false, ...patch,
});
const status = (categories: AccountSyncCategory[], patch: Partial<AccountSyncStatus> = {}): AccountSyncStatus => ({
  signed_in: true, anywhere_enabled: true, last_sync_at: NOW - 120_000, last_error: null,
  categories, sign_ins: [], missing_tools: [], ...patch,
});

describe("classifyAccountSyncError", () => {
  it("maps a 404 to the update hint", () => {
    expect(classifyAccountSyncError(new ApiError(404, "pairing invalid, re-scan")).kind).toBe("old-daemon");
    expect(classifyAccountSyncError(new ApiError(0, "server unreachable")).kind).toBe("unreachable");
    expect(classifyAccountSyncError(new ApiError(500, "boom"))).toEqual({ kind: "error", message: "boom" });
    expect(classifyAccountSyncError("x").kind).toBe("error");
  });
});

describe("toggleLockReason", () => {
  const keys = category({ id: "keys", toggle_requires_direct: true });
  it("locks keys over Anywhere and on phones, never other categories", () => {
    expect(toggleLockReason(keys, { viaAnywhere: true, isPhone: false })).toBe("Only on the computer itself");
    expect(toggleLockReason(keys, { viaAnywhere: false, isPhone: true })).toBe("Only on the computer itself");
    expect(toggleLockReason(keys, { viaAnywhere: false, isPhone: false })).toBeNull();
    expect(toggleLockReason(category(), { viaAnywhere: true, isPhone: true })).toBeNull();
  });
});

describe("categoryStatusLine", () => {
  it("describes off, synced, pending and conflicted categories", () => {
    expect(categoryStatusLine(category(), NOW).text).toBe("Off on this machine");
    const on = category({ enabled: true, items: 1, last_sync_at: NOW - 120_000 });
    expect(categoryStatusLine(on, NOW)).toEqual({ text: "1 item · synced 2m ago", tone: "success" });
    expect(categoryStatusLine({ ...on, items: 4, pending: 2, remote_only: 3 }, NOW).text).toBe("4 items · 2 pending · 3 to apply · synced 2m ago");
    expect(categoryStatusLine({ ...on, conflicts: [{ item: "theme", detail: "x" }] }, NOW).tone).toBe("warn");
    expect(categoryStatusLine({ ...on, last_sync_at: null }, NOW).text).toContain("synced never");
  });
});

describe("syncHubMeta", () => {
  it("summarises the account state", () => {
    expect(syncHubMeta(undefined)).toBe("—");
    expect(syncHubMeta(status([category()], { signed_in: false }))).toBe("Sign in to enable");
    expect(syncHubMeta(status([category(), category({ id: "mcp" })]))).toBe("Off");
    expect(syncHubMeta(status([category({ enabled: true }), category({ id: "mcp", enabled: true }), category({ id: "settings", enabled: true })]), NOW)).toBe("3 on · synced 2m ago");
    expect(syncHubMeta(status([category({ enabled: true, conflicts: [{ item: "a", detail: "b" }] })]))).toBe("1 conflict");
    expect(syncHubMeta(status([category({ enabled: true })], { last_error: "offline" }))).toBe("Needs attention");
  });
});

describe("misc", () => {
  it("formats ages, attention and optimistic toggles", () => {
    expect(ageLabel(null)).toBe("never");
    expect(ageLabel(NOW - 7_200_000, NOW)).toBe("2h ago");
    expect(syncNeedsAttention(status([category({ enabled: true, conflicts: [{ item: "a", detail: "b" }] })]))).toBe(true);
    expect(syncNeedsAttention(status([category()], { last_error: "x" }))).toBe(false);
    const next = withCategoryEnabled(status([category()]), "skills", true);
    expect(next.categories[0].enabled).toBe(true);
  });
});

describe("syncPillModel", () => {
  const on = (patch: Partial<AccountSyncCategory> = {}) => category({ enabled: true, ...patch });
  it("hides when signed out or unknown", () => {
    expect(syncPillModel(undefined)).toBeNull();
    expect(syncPillModel(status([on()], { signed_in: false }))).toBeNull();
  });
  it("covers every state in priority order", () => {
    expect(syncPillModel(status([category()]))?.label).toBe("Sync off");
    expect(syncPillModel(status([on()]), NOW)).toMatchObject({ label: "Synced", tone: "success" });
    expect(syncPillModel(status([on()]), NOW)?.description).toContain("2m ago");
    expect(syncPillModel(status([on({ pending: 2 }), on({ id: "mcp", pending: 1 })]))).toMatchObject({ label: "3 pending", tone: "neutral" });
    expect(syncPillModel(status([on({ conflicts: [{ item: "a", detail: "b" }] })]))).toMatchObject({ label: "1 conflict", tone: "warn" });
    expect(syncPillModel(status([on()], { sign_ins: [{ provider: "codex", label: "Codex", command: "x" }] }))).toMatchObject({ label: "Sign in needed", tone: "warn" });
    expect(syncPillModel(status([on({ conflicts: [{ item: "a", detail: "b" }] })], { last_error: "offline" }))).toMatchObject({ label: "Sync error", tone: "danger" });
    expect(syncPillModel(status([category({ pending: 4 })]))?.label).toBe("Sync off");
  });
});
