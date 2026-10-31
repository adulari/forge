import { describe, expect, it } from "vitest";

import type { UsageActivityDay } from "./api";
import { activityProviders, buildHeatmap, computeStreaks, filterProvider, offsetChanges, summarize } from "./usageActivity";

const row = (day: string, provider: string, tokens: number): UsageActivityDay => ({ day, provider, inputTokens: tokens, outputTokens: 0, costUsd: 0.5, calls: 2 });
const today = new Date(2026, 2, 18, 9);

describe("usageActivity", () => {
  it("counts a streak that ended yesterday as current and a gap as a reset", () => {
    expect(computeStreaks(["2026-03-14", "2026-03-15", "2026-03-17"], "2026-03-18")).toEqual({ current: 1, longest: 2 });
    expect(computeStreaks(["2026-03-10"], "2026-03-18")).toEqual({ current: 0, longest: 1 });
    expect(computeStreaks(["2028-02-28", "2028-02-29", "2028-03-01"], "2028-03-01")).toEqual({ current: 3, longest: 3 });
  });

  it("builds 52 Monday-first columns, hides future days and scales levels to the busiest day", () => {
    const rows = [row("2026-03-16", "a", 100), row("2026-03-17", "a", 400), row("2025-04-01", "b", 10)];
    const weeks = buildHeatmap(rows, today, "all");
    expect(weeks).toHaveLength(52);
    const last = weeks[51];
    expect(last[0]?.day).toBe("2026-03-16");
    expect(last[2]?.day).toBe("2026-03-18");
    expect(last[3]).toBeNull();
    expect(last[1]?.level).toBe(4);
    expect(last[0]?.level).toBe(1);
    expect(last[2]?.level).toBe(0);
  });

  it("marks days outside the 7-day window and summarizes only inside it", () => {
    const rows = [row("2026-03-17", "a", 400), row("2026-03-01", "b", 900)];
    const week = buildHeatmap(rows, today, "7").flat().filter(Boolean);
    expect(week.find((cell) => cell?.day === "2026-03-01")?.inWindow).toBe(false);
    const s = summarize(rows, today, "7");
    expect(s.tokens).toBe(400);
    expect(s.activeDays).toBe(1);
    expect(s.busiest).toEqual({ day: "2026-03-17", tokens: 400 });
    expect(summarize(rows, today, "all").tokens).toBe(1300);
  });

  it("filters by account and orders accounts by usage", () => {
    const rows = [row("2026-03-17", "a", 1), row("2026-03-17", "b", 5)];
    expect(activityProviders(rows)).toEqual(["b", "a"]);
    expect(filterProvider(rows, "a")).toHaveLength(1);
    expect(summarize(filterProvider(rows, "b"), today, "30").providers).toEqual([{ provider: "b", tokens: 5 }]);
  });
});

describe("offsetChanges", () => {
  it("reports one entry for a zone without DST", () => {
    expect(offsetChanges(Date.UTC(2026, 9, 31), 365, () => 3600)).toBe("0:3600");
  });
  it("pins the instant the offset changed", () => {
    const change = Date.UTC(2026, 9, 25, 1, 0, 0);
    const offsetAt = (ms: number) => (ms < change ? 7200 : 3600);
    const out = offsetChanges(Date.UTC(2026, 9, 31), 365, offsetAt).split(",");
    expect(out[0]).toBe("0:7200");
    expect(out).toHaveLength(2);
    const [epoch, offset] = out[1].split(":").map(Number);
    expect(offset).toBe(3600);
    expect(Math.abs(epoch * 1000 - change)).toBeLessThanOrEqual(1000);
  });
});
