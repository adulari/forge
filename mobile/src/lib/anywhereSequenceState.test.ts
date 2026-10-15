import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DebouncedPersister, LiveReplayWindows, OutboundSequenceBlock } from "./anywhereSequenceState";
import type { StoredAnywhereCredentials } from "./transport/anywhereCredentialTypes";

function creds(overrides: Partial<StoredAnywhereCredentials> = {}): StoredAnywhereCredentials {
  return { accountIdHex: "aa", deviceIdHex: "bb", nextSequence: "0", acceptedSequences: {}, ...overrides } as StoredAnywhereCredentials;
}

function blockHarness(initial: bigint | null) {
  const state = { next: initial, writes: [] as bigint[] };
  const io = {
    persistedNext: () => state.next,
    persistNext: async (n: bigint) => { state.next = n; state.writes.push(n); },
    serialize: <T,>(work: () => Promise<T>) => work(),
  };
  return { state, io };
}

describe("OutboundSequenceBlock", () => {
  it("persists once per block and hands out unique increasing sequences", async () => {
    const { state, io } = blockHarness(5n);
    const block = new OutboundSequenceBlock(4n);
    const got: bigint[] = [];
    for (let i = 0; i < 9; i += 1) got.push(await block.reserve(io));
    expect(got).toEqual([5n, 6n, 7n, 8n, 9n, 10n, 11n, 12n, 13n]);
    expect(state.writes).toEqual([9n, 13n, 17n]);
  });

  it("serves concurrent reservations without reuse", async () => {
    const { state, io } = blockHarness(0n);
    let tail = Promise.resolve();
    io.serialize = <T,>(work: () => Promise<T>) => {
      const run = tail.then(work);
      tail = run.then(() => undefined);
      return run;
    };
    const block = new OutboundSequenceBlock(3n);
    const got = await Promise.all(Array.from({ length: 10 }, () => block.reserve(io)));
    expect(new Set(got.map(String)).size).toBe(10);
    expect(state.next! >= 10n).toBe(true);
  });

  it("starts after a sequence another writer consumed past the block limit", async () => {
    const { state, io } = blockHarness(0n);
    const block = new OutboundSequenceBlock(2n);
    expect(await block.reserve(io)).toBe(0n);
    expect(await block.reserve(io)).toBe(1n);
    state.next = 3n; // pairing approval used 2 and persisted 3
    expect(await block.reserve(io)).toBe(3n);
  });

  it("drops the block when credentials are replaced with a lower nextSequence", async () => {
    const { state, io } = blockHarness(100n);
    const block = new OutboundSequenceBlock(10n);
    expect(await block.reserve(io)).toBe(100n);
    state.next = 0n; // new key epoch resets the namespace
    expect(await block.reserve(io)).toBe(0n);
    expect(state.next).toBe(10n);
  });

  it("fails when signed out and a fresh instance never inherits a cursor", async () => {
    const block = new OutboundSequenceBlock(2n);
    await expect(block.reserve(blockHarness(null).io)).rejects.toThrow("not signed in");
    const { io } = blockHarness(50n);
    expect(await new OutboundSequenceBlock(2n).reserve(io)).toBe(50n);
  });
});

describe("LiveReplayWindows", () => {
  it("accepts in memory and rejects in-session replays", () => {
    const windows = new LiveReplayWindows();
    windows.adopt(creds());
    expect(windows.accept("peer", 1, [10n])).toBe(true);
    expect(windows.accept("peer", 1, [10n])).toBe(false);
    expect(windows.accept("peer", 1, [9n, 11n])).toBe(true);
    expect(windows.accept("peer", 2, [10n])).toBe(true);
  });

  it("rejects everything while no credentials are adopted", () => {
    expect(new LiveReplayWindows().accept("peer", 1, [1n])).toBe(false);
  });

  it("keeps live windows when a stale snapshot is adopted", () => {
    const windows = new LiveReplayWindows();
    const stale = creds();
    windows.adopt(stale);
    windows.accept("peer", 1, [7n]);
    const merged = windows.adopt({ ...stale, accessToken: "new" } as StoredAnywhereCredentials);
    expect(merged.acceptedSequences["peer:1"]).toEqual(["7"]);
    expect(windows.accept("peer", 1, [7n])).toBe(false);
  });

  it("starts over for a different device or after reset", () => {
    const windows = new LiveReplayWindows();
    windows.adopt(creds());
    windows.accept("peer", 1, [7n]);
    windows.adopt(creds({ deviceIdHex: "cc" }));
    expect(windows.accept("peer", 1, [7n])).toBe(true);
    windows.reset();
    expect(windows.accept("peer", 1, [8n])).toBe(false);
  });
});

describe("DebouncedPersister", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("writes once per burst after the delay", async () => {
    const write = vi.fn(async () => undefined);
    const persister = new DebouncedPersister(write, 1500);
    for (let i = 0; i < 50; i += 1) persister.markDirty();
    expect(write).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1500);
    expect(write).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(write).toHaveBeenCalledTimes(1);
    persister.markDirty();
    await vi.advanceTimersByTimeAsync(1500);
    expect(write).toHaveBeenCalledTimes(2);
  });

  it("flush writes immediately and cancels the timer", async () => {
    const write = vi.fn(async () => undefined);
    const persister = new DebouncedPersister(write, 1500);
    persister.markDirty();
    await persister.flush();
    expect(write).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(write).toHaveBeenCalledTimes(1);
    await persister.flush();
    expect(write).toHaveBeenCalledTimes(1);
  });

  it("skips the write when a full credential save already covered the change", async () => {
    const write = vi.fn(async () => undefined);
    const persister = new DebouncedPersister(write, 1500);
    persister.markDirty();
    persister.markSaved(persister.version);
    await vi.advanceTimersByTimeAsync(5000);
    expect(write).not.toHaveBeenCalled();
  });

  it("still writes changes that arrived while a full save was in flight", async () => {
    const write = vi.fn(async () => undefined);
    const persister = new DebouncedPersister(write, 1500);
    persister.markDirty();
    const started = persister.version;
    persister.markDirty();
    persister.markSaved(started);
    await vi.advanceTimersByTimeAsync(1500);
    expect(write).toHaveBeenCalledTimes(1);
  });
});
