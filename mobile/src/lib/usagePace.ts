import { formatDurationShort, type DisplayQuota } from "./usageWindows";

// Window lengths for the quota kinds the wire sends (api.ts UsageQuota); "secondary" has no
// fixed length, so it is never projected. "monthly" is OpenCode Go's calendar window, 30 days
// nominal like nominal_window_secs in Rust.
const WEEK_SEC = 7 * 24 * 3600;
export const WINDOW_DURATION_SEC: Record<string, number> = {
  five_hour: 5 * 3600,
  weekly: WEEK_SEC,
  weekly_fable: WEEK_SEC,
  weekly_opus: WEEK_SEC,
  weekly_sonnet: WEEK_SEC,
  monthly: 30 * 24 * 3600,
};

// A burn rate read off the first sliver of a window is noise, so an unflagged quota is only
// projected once this share of the window has elapsed.
const MIN_ELAPSED_SHARE = 0.05;

export interface PaceAnnotation {
  text: string;
  tone: "warn" | "danger";
}

/**
 * Pace-dot annotation ("≈75% by reset" warn / "runs out 1d 8h" danger): a linear extrapolation
 * of the current fraction over the elapsed part of the window. It never echoes the current
 * fraction as a forecast. The daemon's own "warning"/"exhausted" status always earns a
 * projection; an "ok" quota gets one only when the burn rate would empty it before the reset.
 */
export function paceAnnotation(quota: DisplayQuota, nowSec: number): PaceAnnotation | null {
  const duration = WINDOW_DURATION_SEC[quota.windowKind];
  if (!duration || quota.fraction == null || quota.resetsAt == null) return null;
  const remainingSec = quota.resetsAt - nowSec;
  const elapsedSec = duration - remainingSec;
  if (remainingSec <= 0 || elapsedSec <= 0) return null;
  const rate = quota.fraction / elapsedSec;
  if (rate <= 0) return null;
  const flagged = quota.status === "warning" || quota.status === "exhausted";
  if (!flagged && elapsedSec < duration * MIN_ELAPSED_SHARE) return null;
  if (quota.fraction < 1) {
    const secToExhaust = (1 - quota.fraction) / rate;
    if (secToExhaust < remainingSec) {
      return { text: `runs out ${formatDurationShort(secToExhaust)}`, tone: "danger" };
    }
  }
  if (!flagged) return null;
  const projected = rate * duration;
  return { text: `≈${Math.round(Math.min(999, projected * 100))}% by reset`, tone: "warn" };
}
