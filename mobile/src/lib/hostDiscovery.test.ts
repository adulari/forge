import { beforeEach, describe, expect, it, vi } from "vitest";

import { onRediscoverRequested, requestHostRediscovery, resetHostDiscoveryForTests } from "./hostDiscovery";

describe("requestHostRediscovery", () => {
  beforeEach(resetHostDiscoveryForTests);

  it("notifies listeners once per cooldown however many sockets ask", () => {
    const listener = vi.fn();
    onRediscoverRequested(listener);
    expect(requestHostRediscovery(1_000_000)).toBe(true);
    expect(requestHostRediscovery(1_000_100)).toBe(false);
    expect(requestHostRediscovery(1_009_999)).toBe(false);
    expect(listener).toHaveBeenCalledTimes(1);
    expect(requestHostRediscovery(1_010_000)).toBe(true);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("stops notifying an unsubscribed listener", () => {
    const listener = vi.fn();
    const off = onRediscoverRequested(listener);
    off();
    requestHostRediscovery(5_000_000);
    expect(listener).not.toHaveBeenCalled();
  });
});
