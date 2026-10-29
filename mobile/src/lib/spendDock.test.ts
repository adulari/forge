import { describe, expect, it } from "vitest";

import type { SpendTotals } from "./api";
import { hasSpend, spendDetail, spendHeadline, totalTokens } from "./spendDock";

const totals = (over: Partial<SpendTotals> = {}): SpendTotals => ({
  input_tokens: 12_000,
  cached_input_tokens: null,
  output_tokens: 1_500,
  calls: 4,
  cost_usd: 0.42,
  estimate_usd: 0.8,
  ...over,
});

describe("spend readouts", () => {
  it("leads with billed spend", () => {
    expect(spendHeadline(totals())).toBe("$0.42");
  });

  it("marks a list-price estimate so it never reads as money spent", () => {
    expect(spendHeadline(totals({ cost_usd: 0 }))).toBe("≈$0.80 list");
    expect(spendHeadline(totals({ cost_usd: 0, estimate_usd: null }))).toBe("$0.00");
  });

  it("details tokens and calls, adding the list price only when it differs from the bill", () => {
    expect(spendDetail(totals())).toBe("12k in · 1.5k out · 4 calls · ≈$0.80 list");
    expect(spendDetail(totals({ estimate_usd: 0.42, calls: 1 }))).toBe("12k in · 1.5k out · 1 call");
  });

  it("counts a session as spending once it has called a provider", () => {
    expect(hasSpend(undefined)).toBe(false);
    expect(hasSpend(totals({ calls: 0, input_tokens: 0, output_tokens: 0 }))).toBe(false);
    expect(hasSpend(totals())).toBe(true);
    expect(totalTokens(totals())).toBe(13_500);
  });
});
