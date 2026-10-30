// Pure shaping for the Usage Activity tab, the share card and the Settings activity card: the
// daemon sends one row per local day and provider, everything else is derived here so the tab,
// the PNG and the TUI poster (`forge stats poster`) agree.
import type { UsageActivityDay } from "./api";

export type ActivityWindow = "7" | "30" | "all";
export const ACTIVITY_WEEKS = 52;
const DAY_MS = 86_400_000;

export interface HeatCell {
  day: string;
  tokens: number;
  /** 0 = no activity, 1-4 = share of the busiest day. */
  level: 0 | 1 | 2 | 3 | 4;
  inWindow: boolean;
}

export interface ActivitySummary {
  tokens: number;
  calls: number;
  costUsd: number;
  activeDays: number;
  currentStreak: number;
  longestStreak: number;
  busiest: { day: string; tokens: number } | null;
  providers: { provider: string; tokens: number }[];
}

export const dayKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;

const parseDay = (day: string) => {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d, 12);
};

export const windowDays = (window: ActivityWindow) => (window === "7" ? 7 : window === "30" ? 30 : ACTIVITY_WEEKS * 7);

export function filterProvider(rows: UsageActivityDay[], provider: string | null): UsageActivityDay[] {
  return provider ? rows.filter((row) => row.provider === provider) : rows;
}

export function activityProviders(rows: UsageActivityDay[]): string[] {
  const totals = new Map<string, number>();
  for (const row of rows) totals.set(row.provider, (totals.get(row.provider) ?? 0) + row.inputTokens + row.outputTokens);
  return [...totals.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([provider]) => provider);
}

export function perDayTokens(rows: UsageActivityDay[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const row of rows) out.set(row.day, (out.get(row.day) ?? 0) + row.inputTokens + row.outputTokens);
  return out;
}

export function computeStreaks(activeDays: Iterable<string>, today: string): { current: number; longest: number } {
  const ordinals = [...new Set(activeDays)].map((day) => Math.round(parseDay(day).getTime() / DAY_MS)).sort((a, b) => a - b);
  let run = 0;
  let longest = 0;
  let previous: number | null = null;
  for (const ordinal of ordinals) {
    run = previous === ordinal - 1 ? run + 1 : 1;
    longest = Math.max(longest, run);
    previous = ordinal;
  }
  const todayOrdinal = Math.round(parseDay(today).getTime() / DAY_MS);
  const current = previous !== null && (previous === todayOrdinal || previous === todayOrdinal - 1) ? run : 0;
  return { current, longest };
}

const level = (tokens: number, max: number): HeatCell["level"] => {
  if (tokens <= 0 || max <= 0) return 0;
  return Math.min(4, Math.max(1, Math.ceil((tokens / max) * 4))) as HeatCell["level"];
};

/** 52 week columns, Monday first, ending on the week that holds `today`. Days after today are null. */
export function buildHeatmap(rows: UsageActivityDay[], today: Date, window: ActivityWindow): (HeatCell | null)[][] {
  const perDay = perDayTokens(rows);
  const max = Math.max(0, ...perDay.values());
  const mondayOffset = (today.getDay() + 6) % 7;
  const start = new Date(today.getFullYear(), today.getMonth(), today.getDate() - mondayOffset - (ACTIVITY_WEEKS - 1) * 7, 12);
  const windowStart = new Date(today.getFullYear(), today.getMonth(), today.getDate() - windowDays(window) + 1, 12);
  const todayKey = dayKey(today);
  const weeks: (HeatCell | null)[][] = [];
  for (let week = 0; week < ACTIVITY_WEEKS; week += 1) {
    const column: (HeatCell | null)[] = [];
    for (let row = 0; row < 7; row += 1) {
      const date = new Date(start.getFullYear(), start.getMonth(), start.getDate() + week * 7 + row, 12);
      const key = dayKey(date);
      if (key > todayKey) {
        column.push(null);
        continue;
      }
      const tokens = perDay.get(key) ?? 0;
      column.push({ day: key, tokens, level: level(tokens, max), inWindow: date >= windowStart });
    }
    weeks.push(column);
  }
  return weeks;
}

export function summarize(rows: UsageActivityDay[], today: Date, window: ActivityWindow): ActivitySummary {
  const todayKey = dayKey(today);
  const first = dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() - windowDays(window) + 1, 12));
  const inWindow = rows.filter((row) => row.day >= first && row.day <= todayKey);
  const perDay = perDayTokens(inWindow);
  const providers = new Map<string, number>();
  let calls = 0;
  let costUsd = 0;
  for (const row of inWindow) {
    providers.set(row.provider, (providers.get(row.provider) ?? 0) + row.inputTokens + row.outputTokens);
    calls += row.calls;
    costUsd += row.costUsd;
  }
  const { current, longest } = computeStreaks(perDayTokens(rows).keys(), todayKey);
  const busiest = [...perDay.entries()].sort((a, b) => b[1] - a[1] || b[0].localeCompare(a[0]))[0];
  return {
    tokens: [...perDay.values()].reduce((sum, value) => sum + value, 0),
    calls,
    costUsd,
    activeDays: perDay.size,
    currentStreak: current,
    longestStreak: longest,
    busiest: busiest ? { day: busiest[0], tokens: busiest[1] } : null,
    providers: [...providers.entries()].map(([provider, tokens]) => ({ provider, tokens })).sort((a, b) => b.tokens - a.tokens),
  };
}

export const compactTokens = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value).toLowerCase();
