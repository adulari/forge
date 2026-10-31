import { describe, expect, it } from "vitest";

import { msUntilNextLocalDay } from "./useToday";

describe("msUntilNextLocalDay", () => {
  it("lands one second after the coming local midnight", () => {
    expect(msUntilNextLocalDay(new Date(2026, 9, 31, 23, 59, 0))).toBe(61_000);
    expect(msUntilNextLocalDay(new Date(2026, 9, 31, 0, 0, 0))).toBe(24 * 3600 * 1000 + 1000);
  });
  it("never schedules a busy loop", () => {
    expect(msUntilNextLocalDay(new Date(2026, 9, 31, 23, 59, 59, 999))).toBeGreaterThanOrEqual(1000);
  });
});
