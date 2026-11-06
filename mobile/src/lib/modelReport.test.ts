import { describe, expect, it } from "vitest";

import type { ModelReportEntry } from "./api";
import { costLabel, latencyLabel, pct, reportById, reportLine, sortByReport } from "./modelReport";

const entry = (model: string, over: Partial<ModelReportEntry> = {}): ModelReportEntry => ({
  model,
  calls: 10,
  completed: 9,
  failed: 1,
  failed_over: 0,
  median_latency_ms: 1300,
  median_cost_usd: 0.015,
  cost_samples: 10,
  tool_calls: 10,
  tool_errors: 2,
  completion_rate: 0.9,
  tool_error_rate: 0.2,
  benchmark_intelligence: null,
  benchmark_coding: null,
  ...over,
});

describe("model report formatting", () => {
  it("formats rates, latency and cost", () => {
    expect(pct(0.934)).toBe("93%");
    expect(pct(null)).toBe("—");
    expect(latencyLabel(850)).toBe("850ms");
    expect(latencyLabel(2400)).toBe("2.4s");
    expect(costLabel(0)).toBe("$0");
    expect(costLabel(0.0042)).toBe("$0.0042");
    expect(costLabel(0.015)).toBe("$0.015");
    expect(costLabel(0.25)).toBe("$0.25");
  });

  it("summarises a model and returns null when it has no data", () => {
    expect(reportLine(entry("a"))).toBe("90% done · 10 calls · 1 failed · 1.3s · $0.015 · tool err 20%");
    expect(reportLine(entry("a", { failed: 0, failed_over: 3, calls: 1 }))).toContain("3 failed (3 failed over)");
    expect(reportLine(entry("a", { calls: 0 }))).toBeNull();
    expect(reportLine(undefined)).toBeNull();
  });
});

describe("sortByReport", () => {
  const report = reportById([
    entry("flaky", { completion_rate: 0.1, median_cost_usd: 0.001 }),
    entry("solid", { completion_rate: 0.98, median_cost_usd: 0.05 }),
    entry("free", { completion_rate: 0.9, median_cost_usd: 0 }),
  ]);
  const ids = ["none", "flaky", "solid", "free"];

  it("puts the most reliable first and models with no data last", () => {
    expect(sortByReport(ids, (id) => id, report, "completion")).toEqual(["solid", "free", "flaky", "none"]);
  });

  it("puts the cheapest first and models with no data last", () => {
    expect(sortByReport(ids, (id) => id, report, "cost")).toEqual(["free", "flaky", "solid", "none"]);
  });
});
