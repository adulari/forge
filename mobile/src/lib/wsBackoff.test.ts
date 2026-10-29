import { describe, expect, it } from "vitest";

import { BACKOFF_MS, backoffDelay, shouldRediscover } from "./wsBackoff";

describe("backoffDelay", () => {
  it("stays within 25% of the schedule", () => {
    for (let attempt = 0; attempt < BACKOFF_MS.length; attempt++) {
      const low = backoffDelay(attempt, () => 0);
      const high = backoffDelay(attempt, () => 1);
      expect(low).toBe(Math.round(BACKOFF_MS[attempt] * 0.75));
      expect(high).toBe(Math.round(BACKOFF_MS[attempt] * 1.25));
      expect(backoffDelay(attempt, () => 0.5)).toBe(BACKOFF_MS[attempt]);
    }
  });

  it("clamps to the last step", () => {
    expect(backoffDelay(99, () => 0.5)).toBe(15000);
    expect(backoffDelay(-3, () => 0.5)).toBe(500);
  });

  it("spreads sockets that fail together", () => {
    const delays = new Set(Array.from({ length: 20 }, (_, i) => backoffDelay(3, () => i / 19)));
    expect(delays.size).toBeGreaterThan(10);
  });
});

describe("shouldRediscover", () => {
  it("fires on every third consecutive failure", () => {
    expect([0, 1, 2, 3, 4, 5, 6].filter(shouldRediscover)).toEqual([3, 6]);
  });
});
