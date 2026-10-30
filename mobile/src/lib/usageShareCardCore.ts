import { compactTokens, type ActivitySummary, type HeatCell } from "./usageActivity";

export interface ShareCardInput {
  summary: ActivitySummary;
  weeks: (HeatCell | null)[][];
  windowLabel: string;
  accountLabel: string;
  palette: { bg: string; ink: string; ink3: string; accent: string; track: string; levels: [string, string, string, string] };
}

export function shareCardLines(input: ShareCardInput): string[] {
  const { summary } = input;
  return [
    `Forge · ${input.windowLabel} · ${input.accountLabel}`,
    `${compactTokens(summary.tokens)} tokens · ${summary.activeDays} active days`,
    `Streak ${summary.currentStreak}d · longest ${summary.longestStreak}d`,
    `${compactTokens(summary.calls)} model calls · $${summary.costUsd.toFixed(2)} at list price, not a bill`,
  ];
}
