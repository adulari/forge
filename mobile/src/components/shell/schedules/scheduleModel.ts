// Pure helpers behind the Schedules panel: when a row fires next, what its last run said, and
// which rows the filter + search keep. No React, so the rules are unit-tested directly.
import type { ScheduleRow, ScheduleRun, ScheduleRunStatus } from "../../../lib/api";
import { describeStoredSpec, nextCronRun } from "../../../lib/cron";
import { formatInstant, nextCronRunInZone } from "../../../lib/cronZone";

/** The daemon serialises timestamps in seconds on some rows and milliseconds on others; a
 * value below year-2001-in-ms can only be seconds. */
export function epochMs(value: number | null): number | null {
  if (value == null || value <= 0) return null;
  return value < 1e12 ? value * 1000 : value;
}

/** The slice of a row that decides when it fires - also what the form previews before saving. */
export type TimedSchedule = Pick<ScheduleRow, "cron" | "spec_label" | "enabled" | "tz" | "last_run" | "created_at">;

/** `4m`, `3h`, `2d` - kept local so this module stays free of react-native imports. */
export function ago(fromMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.round((nowMs - fromMs) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  return hours < 48 ? `${hours}h` : `${Math.round(hours / 24)}d`;
}

/**
 * Next fire time for a stored spec. `every:<seconds>` is arithmetic from the last tick,
 * `daily:HH:MM` and `cron:<expr>` are calendar walks in the schedule's own zone (the viewer's
 * when it has none). A systemd `OnCalendar=` row returns null: a guessed "next" would be worse
 * than none.
 */
export function nextRunMs(row: TimedSchedule, now: number): number | null {
  if (!row.enabled) return null;
  const zone = row.tz?.trim() || null;

  if (row.cron.startsWith("every:")) {
    const seconds = parseInt(row.cron.slice("every:".length), 10);
    if (!Number.isFinite(seconds) || seconds <= 0) return null;
    const period = seconds * 1000;
    const base = epochMs(row.last_run) ?? epochMs(row.created_at) ?? now;
    return base + Math.max(1, Math.ceil((now - base) / period)) * period;
  }

  let expression: string | null = null;
  if (row.cron.startsWith("daily:")) {
    const [hours, minutes] = row.cron.slice("daily:".length).split(":").map((part) => parseInt(part, 10));
    if (!Number.isFinite(hours) || !Number.isFinite(minutes)) return null;
    expression = `${minutes} ${hours} * * *`;
  } else if (row.cron.startsWith("cron:")) {
    expression = row.cron.slice("cron:".length);
  }
  if (expression === null) return null;
  return zone ? nextCronRunInZone(expression, now, zone) : nextCronRun(expression, now);
}

/** "Weekdays at 3:00 AM" - the cadence in words, with the zone appended when it is not local. */
export function cadenceLabel(row: TimedSchedule): string {
  const words = describeStoredSpec(row.cron, row.spec_label);
  return row.tz ? `${words} (${row.tz})` : words;
}

/** "Next run Jul 30, 3:00 AM", or why there is none. */
export function nextRunLabel(row: TimedSchedule, now: number): string {
  if (!row.enabled) return "Paused";
  const next = nextRunMs(row, now);
  if (next != null) return `Next run ${formatInstant(next, row.tz?.trim() || undefined)}`;
  const last = epochMs(row.last_run);
  return last ? `Last run ${ago(last, now)} ago` : "Not run yet";
}

export const RUN_STATUS_LABEL: Record<ScheduleRunStatus, string> = {
  running: "Running",
  ok: "Succeeded",
  failed: "Failed",
  interrupted: "Interrupted",
  skipped: "Skipped",
};

/** One line for the row's last result: "Failed 4m ago - stopped after hitting the 600s limit". */
export function lastResultLine(run: ScheduleRun | null, now: number): string | null {
  if (!run) return null;
  const when = ago(epochMs(run.finished_at ?? run.started_at) ?? now, now);
  const head = run.status === "running" ? `Running since ${when} ago` : `${RUN_STATUS_LABEL[run.status]} ${when} ago`;
  const detail = (run.status === "ok" ? run.summary : run.error ?? run.summary)?.replace(/\s+/g, " ").trim();
  return detail ? `${head} - ${detail}` : head;
}

export type ScheduleFilter = "all" | "active" | "paused";

export function filterSchedules(rows: ScheduleRow[], filter: ScheduleFilter, query: string): ScheduleRow[] {
  const needle = query.trim().toLowerCase();
  return rows.filter((row) => {
    if (filter === "active" && !row.enabled) return false;
    if (filter === "paused" && row.enabled) return false;
    if (!needle) return true;
    return [row.task, row.cwd, row.model ?? "", row.spec_label, cadenceLabel(row)]
      .some((field) => field.toLowerCase().includes(needle));
  });
}

export interface ScheduleTemplate {
  id: string;
  title: string;
  description: string;
  task: string;
  cadence: { kind: "daily" | "weekdays" | "weekly"; time: string; weekday?: number };
}

export const SCHEDULE_TEMPLATES: ScheduleTemplate[] = [
  {
    id: "daily-brief",
    title: "Daily brief",
    description: "Every weekday at 8:00 AM",
    task: "Summarise what changed in this repository since yesterday: merged work, open pull requests and failing checks. Keep it to ten lines.",
    cadence: { kind: "weekdays", time: "08:00" },
  },
  {
    id: "dependency-audit",
    title: "Dependency audit",
    description: "Mondays at 9:00 AM",
    task: "Audit the dependencies for known vulnerabilities and outdated majors. List what to upgrade first and why; do not change any files.",
    cadence: { kind: "weekly", time: "09:00", weekday: 1 },
  },
  {
    id: "weekly-changelog",
    title: "Weekly changelog",
    description: "Fridays at 4:00 PM",
    task: "Draft a changelog for this week from the merged commits, grouped by area, in plain language a user would read.",
    cadence: { kind: "weekly", time: "16:00", weekday: 5 },
  },
];

/** "Weekdays at 3:00 AM (Asia/Tokyo)  ·  Next run Jul 30, 3:00 AM" for a spec not saved yet. */
export function previewLabel(stored: string, tz: string | null, now: number): string {
  const row: TimedSchedule = { cron: stored, spec_label: stored, enabled: true, tz, last_run: null, created_at: now / 1000 };
  return `${cadenceLabel(row)}  ·  ${nextRunLabel(row, now)}`;
}
