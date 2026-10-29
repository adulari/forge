import type { SpendTotals } from "./api";
import { compactTokens } from "./goalState";

/** Kept here, not imported from the theme, so the module stays free of react-native. */
function formatCost(usd: number): string {
  if (usd === 0) return "$0.00";
  return Math.abs(usd) < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

export function totalTokens(totals: Pick<SpendTotals, "input_tokens" | "output_tokens">): number {
  return totals.input_tokens + totals.output_tokens;
}

/** `$0.42` of billed spend, or the list-price estimate when nothing was billed (a subscription or
 * free route), marked so it is never read as money spent. */
export function spendHeadline(totals: SpendTotals): string {
  if (totals.cost_usd > 0) return formatCost(totals.cost_usd);
  if (totals.estimate_usd != null && totals.estimate_usd > 0) return `≈${formatCost(totals.estimate_usd)} list`;
  return formatCost(0);
}

export function spendDetail(totals: SpendTotals): string {
  const parts = [`${compactTokens(totals.input_tokens)} in`, `${compactTokens(totals.output_tokens)} out`];
  if (totals.calls > 0) parts.push(`${totals.calls} ${totals.calls === 1 ? "call" : "calls"}`);
  if (totals.cost_usd > 0 && totals.estimate_usd != null && totals.estimate_usd > totals.cost_usd) {
    parts.push(`≈${formatCost(totals.estimate_usd)} list`);
  }
  return parts.join(" · ");
}

/** Worth a dock: the session made at least one provider call. */
export function hasSpend(totals: SpendTotals | undefined): totals is SpendTotals {
  return totals != null && (totals.calls > 0 || totalTokens(totals) > 0);
}
