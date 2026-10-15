import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionRow } from "../api";
import {
  clearFleetRows, loadFleetRows, peekFleetRows, relayHostIdOf, rememberFleetRows, resetFleetRowCacheForTests,
  type FleetRowStorage,
} from "./fleetRowCache";

vi.mock("@react-native-async-storage/async-storage", () => ({ default: {} }));

const row = (id: string, extra: Record<string, unknown> = {}): SessionRow => ({
  id, title: `t-${id}`, cwd: "/w", worktree: null, busy: false, waiting: false, cost_usd: 0,
  context_tokens: 0, context_limit: null, model: "m", permission_mode: null, created_at: 1, last_activity: 2,
  ...extra,
} as SessionRow);

function memoryStorage(): FleetRowStorage & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: async (key) => data.get(key) ?? null,
    setItem: async (key, value) => { data.set(key, value); },
    removeItem: async (key) => { data.delete(key); },
  };
}

describe("fleet row cache", () => {
  beforeEach(() => { vi.useFakeTimers(); resetFleetRowCacheForTests(); });
  afterEach(() => vi.useRealTimers());

  it("only caches relay hosts", () => {
    expect(relayHostIdOf("fany://abc")).toBe("abc");
    expect(relayHostIdOf("https://secret@host.example")).toBeNull();
    const storage = memoryStorage();
    rememberFleetRows("https://host.example", [row("a")], storage);
    expect(peekFleetRows("https://host.example")).toBeUndefined();
  });

  it("persists debounced, projects away unknown fields, and seeds a fresh process", async () => {
    const storage = memoryStorage();
    rememberFleetRows("fany://h1", [row("a", { transcript: "secret prompt text" }), row("b")], storage);
    rememberFleetRows("fany://h1", [row("a", { transcript: "secret prompt text" }), row("b"), row("c")], storage);
    expect(storage.data.size).toBe(0);
    await vi.advanceTimersByTimeAsync(2000);
    expect(storage.data.size).toBe(1);
    expect([...storage.data.values()][0]).not.toContain("secret prompt text");

    resetFleetRowCacheForTests();
    expect(peekFleetRows("fany://h1")).toBeUndefined();
    await loadFleetRows(["h1", "h2"], storage);
    expect(peekFleetRows("fany://h1")?.map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(peekFleetRows("fany://h2")).toBeUndefined();
  });

  it("ignores corrupt storage and clears on request", async () => {
    const storage = memoryStorage();
    storage.data.set("forge.anywhere.fleetRows.v1.h1", "{not json");
    await loadFleetRows(["h1"], storage);
    expect(peekFleetRows("fany://h1")).toBeUndefined();
    rememberFleetRows("fany://h1", [row("a")], storage);
    await clearFleetRows(["h1"], storage);
    await vi.advanceTimersByTimeAsync(5000);
    expect(storage.data.size).toBe(0);
    expect(peekFleetRows("fany://h1")).toBeUndefined();
  });
});
