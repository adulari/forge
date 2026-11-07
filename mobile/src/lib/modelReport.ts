import type { ModelReportEntry } from "./api";

export type ReportSort = "tier" | "completion" | "cost";

export const REPORT_DAYS = 30;

export function pct(rate: number | null | undefined): string {
  return rate == null ? "—" : `${Math.round(rate * 100)}%`;
}

export function latencyLabel(ms: number | null | undefined): string {
  if (ms == null) return "—";
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`;
}

export function costLabel(usd: number | null | undefined): string {
  if (usd == null) return "—";
  if (usd < 0.00005) return "$0";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  if (usd < 0.1) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

/** Compact line for one model row, e.g. `87% done · 23 calls · 1.3s · $0.015 · tool err 20%`.
 * `null` means the model has no recorded calls in the window (the row shows an empty state). */
export function reportLine(entry: ModelReportEntry | undefined): string | null {
  if (!entry || entry.calls === 0) return null;
  const failed = entry.failed + entry.failed_over;
  return [
    `${pct(entry.completion_rate)} done`,
    `${entry.calls} ${entry.calls === 1 ? "call" : "calls"}`,
    failed > 0 ? `${failed} failed${entry.failed_over > 0 ? ` (${entry.failed_over} failed over)` : ""}` : null,
    latencyLabel(entry.median_latency_ms),
    costLabel(entry.median_cost_usd),
    entry.tool_error_rate == null ? null : `tool err ${pct(entry.tool_error_rate)}`,
  ]
    .filter(Boolean)
    .join(" · ");
}

export function reportById(entries: readonly ModelReportEntry[] | undefined): Map<string, ModelReportEntry> {
  return new Map((entries ?? []).map((entry) => [entry.model, entry]));
}

/** Order ids by the chosen metric; models without data always sort last. Highest completion
 * first, cheapest median cost first; ties break on more calls (more evidence) then id. */
export function sortByReport<T>(items: readonly T[], idOf: (item: T) => string, report: Map<string, ModelReportEntry>, sort: Exclude<ReportSort, "tier">): T[] {
  const key = (item: T): number | null => {
    const entry = report.get(idOf(item));
    if (!entry || entry.calls === 0) return null;
    return sort === "completion" ? -(entry.completion_rate ?? 0) : entry.median_cost_usd;
  };
  return [...items].sort((a, b) => {
    const ka = key(a);
    const kb = key(b);
    if (ka == null || kb == null) return ka == null && kb == null ? idOf(a).localeCompare(idOf(b)) : ka == null ? 1 : -1;
    return ka - kb || (report.get(idOf(b))?.calls ?? 0) - (report.get(idOf(a))?.calls ?? 0) || idOf(a).localeCompare(idOf(b));
  });
}

/** One line for the models page: how the window's turns ended, e.g. `Turn endings: final answer 120 · stalled 3`. */
export function turnEndingsLine(endings: { reason: string; count: number }[] | undefined): string | null {
  if (!endings || endings.length === 0) return null;
  const parts = endings.slice(0, 6).map((e) => `${e.reason.replace(/_/g, " ")} ${e.count}`);
  return `Turn endings: ${parts.join(" · ")}`;
}
