import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  HOST_RESTART_GRACE_MS,
  clearHostRestarting,
  clearStoreRecovery,
  getStoreRecovery,
  isHostRestarting,
  markHostRestarting,
  noteHostResponse,
  onHostCap,
  reportHostCap,
} from "./hostStatus";

describe("host restart window", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    clearHostRestarting();
    vi.useRealTimers();
  });

  it("holds for the grace period and then lapses", () => {
    expect(isHostRestarting()).toBe(false);
    markHostRestarting();
    expect(isHostRestarting()).toBe(true);
    vi.advanceTimersByTime(HOST_RESTART_GRACE_MS - 1);
    expect(isHostRestarting()).toBe(true);
    vi.advanceTimersByTime(2);
    expect(isHostRestarting()).toBe(false);
  });

  it("ends as soon as the host answers again", () => {
    markHostRestarting();
    clearHostRestarting();
    expect(isHostRestarting()).toBe(false);
  });
});

describe("host cap notices", () => {
  it("reaches listeners once per burst of the same message", () => {
    const seen: string[] = [];
    const off = onHostCap((message) => seen.push(message));
    reportHostCap("too many terminals", 10_000);
    reportHostCap("too many terminals", 10_500);
    reportHostCap("too many terminals", 13_000);
    reportHostCap("too many sessions", 13_100);
    off();
    reportHostCap("ignored", 99_000);
    expect(seen).toEqual(["too many terminals", "too many terminals", "too many sessions"]);
  });
});

describe("noteHostResponse", () => {
  it("turns a host_cap refusal into a notice carrying the host's own sentence", () => {
    const seen: string[] = [];
    const off = onHostCap((message) => seen.push(message));
    noteHostResponse("http://h/t", 429, { error: "this host already has 2 live sessions", code: "host_cap" });
    noteHostResponse("http://h/t", 429, { error: "slow down" });
    noteHostResponse("http://h/t", 500, null);
    off();
    expect(seen).toEqual(["this host already has 2 live sessions"]);
  });

  it("flags a store_recovery 503, and only a 503, as the recovery state for that host", () => {
    noteHostResponse("http://h/t", 500, { error: "x", code: "store_recovery" });
    expect(getStoreRecovery()).toBeNull();
    noteHostResponse("http://h/t", 503, { error: "x", code: "store_recovery" });
    expect(getStoreRecovery()).toBe("http://h/t");
    clearStoreRecovery();
    expect(getStoreRecovery()).toBeNull();
  });
});
