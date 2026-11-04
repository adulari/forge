// Presentation rules for the background-jobs sheet (GET /api/sessions/{id}/jobs).
import type { SessionJob } from "../../lib/api";

/** Running jobs first, then finished ones; each group newest first. */
export function orderJobs(jobs: readonly SessionJob[]): SessionJob[] {
  return [...jobs].sort(
    (a, b) => Number(b.running) - Number(a.running) || b.started_at - a.started_at,
  );
}

export function jobAge(startedAt: number, nowSecs: number): string {
  const secs = Math.max(0, nowSecs - startedAt);
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86400)}d`;
}
