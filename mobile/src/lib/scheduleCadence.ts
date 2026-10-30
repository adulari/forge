// The cadence builder's model: a few friendly shapes that each compile to the one-of body the
// daemon already accepts (`every` / `at` / `cron`), and back from a stored spec when editing.
import type { CreateScheduleRequest } from "./api";
import { parseCron } from "./cron";

export type CadenceKind = "hourly" | "daily" | "weekdays" | "weekly" | "once" | "custom";

export interface Cadence {
  kind: CadenceKind;
  /** `HH:MM`, 24-hour: the time of day for daily / weekdays / weekly / once. */
  time: string;
  /** Minute past the hour for hourly. */
  minute: number;
  /** 0-6 (Sunday = 0) for weekly. */
  weekday: number;
  /** `YYYY-MM-DD` for once. */
  date: string;
  /** Raw cron or `every:` form for custom. */
  custom: string;
}

export const CADENCE_OPTIONS: { value: CadenceKind; label: string }[] = [
  { value: "hourly", label: "Hourly" },
  { value: "daily", label: "Daily" },
  { value: "weekdays", label: "Weekdays" },
  { value: "weekly", label: "Weekly" },
  { value: "once", label: "One-time" },
  { value: "custom", label: "Custom" },
];

export function defaultCadence(): Cadence {
  return { kind: "daily", time: "09:00", minute: 0, weekday: 1, date: "", custom: "" };
}

export function parseTime(raw: string): { hour: number; minute: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(raw.trim());
  if (!match) return null;
  const hour = parseInt(match[1], 10);
  const minute = parseInt(match[2], 10);
  return hour <= 23 && minute <= 59 ? { hour, minute } : null;
}

function parseDate(raw: string): { year: number; month: number; day: number } | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(raw.trim());
  if (!match) return null;
  const [year, month, day] = [parseInt(match[1], 10), parseInt(match[2], 10), parseInt(match[3], 10)];
  const probe = new Date(Date.UTC(year, month - 1, day));
  const real = probe.getUTCFullYear() === year && probe.getUTCMonth() === month - 1 && probe.getUTCDate() === day;
  return real ? { year, month, day } : null;
}

/** Accepts the stored forms (`every:1800`, `daily:09:00`, `cron:0 2 * * *`) and the bare ones. */
function customBody(raw: string): Pick<CreateScheduleRequest, "every" | "at" | "cron"> | null {
  const value = raw.trim();
  if (!value) return null;
  const every = (interval: string) => ({ every: /^\d+$/.test(interval) ? `${interval}s` : interval });
  if (value.startsWith("every:")) return every(value.slice("every:".length));
  if (value.startsWith("daily:")) return { at: value.slice("daily:".length) };
  if (value.startsWith("cron:")) return { cron: value.slice("cron:".length).trim() };
  if (/^\d+[smhd]?$/.test(value)) return every(value);
  if (/^\d{1,2}:\d{2}$/.test(value)) return { at: value };
  return { cron: value };
}

export interface CadenceBody {
  body: Pick<CreateScheduleRequest, "every" | "at" | "cron">;
  /** A one-time schedule is a calendar date plus "stop after the first run". */
  maxIterations?: number;
}

/** The request fields this cadence compiles to, or null while the form is incomplete or invalid. */
export function cadenceToBody(cadence: Cadence): CadenceBody | null {
  switch (cadence.kind) {
    case "hourly":
      return Number.isInteger(cadence.minute) && cadence.minute >= 0 && cadence.minute <= 59
        ? { body: { cron: `${cadence.minute} * * * *` } }
        : null;
    case "daily": {
      const time = parseTime(cadence.time);
      return time ? { body: { at: `${String(time.hour).padStart(2, "0")}:${String(time.minute).padStart(2, "0")}` } } : null;
    }
    case "weekdays": {
      const time = parseTime(cadence.time);
      return time ? { body: { cron: `${time.minute} ${time.hour} * * 1-5` } } : null;
    }
    case "weekly": {
      const time = parseTime(cadence.time);
      return time ? { body: { cron: `${time.minute} ${time.hour} * * ${cadence.weekday}` } } : null;
    }
    case "once": {
      const time = parseTime(cadence.time);
      const date = parseDate(cadence.date);
      return time && date
        ? { body: { cron: `${time.minute} ${time.hour} ${date.day} ${date.month} *` }, maxIterations: 1 }
        : null;
    }
    case "custom": {
      const body = customBody(cadence.custom);
      if (!body) return null;
      // Not 5-field cron is only acceptable as a systemd OnCalendar string, which always has a
      // clock time in it.
      if (body.cron !== undefined && parseCron(body.cron) === null && !/\d{1,2}:\d{2}/.test(body.cron)) return null;
      return { body };
    }
  }
}

const pad = (value: number) => String(value).padStart(2, "0");

/** Best-effort inverse of {@link cadenceToBody} for the edit form; anything unrecognised lands in
 * `custom` with the stored spec, so editing never loses a cadence. */
export function cadenceFromStored(stored: string, maxIterations: number | null): Cadence {
  const base = defaultCadence();
  const custom = (): Cadence => ({ ...base, kind: "custom", custom: stored });
  if (stored.startsWith("daily:")) {
    const time = parseTime(stored.slice("daily:".length));
    return time ? { ...base, kind: "daily", time: `${pad(time.hour)}:${pad(time.minute)}` } : custom();
  }
  if (!stored.startsWith("cron:")) return custom();
  const fields = parseCron(stored.slice("cron:".length));
  if (!fields || fields.minutes.length !== 1) return custom();
  const minute = fields.minutes[0];
  const allMonths = fields.months.length === 12;
  const allDom = fields.daysOfMonth.length === 31;
  if (fields.hours.length === 24 && allMonths && allDom && fields.daysOfWeek.length === 7) {
    return { ...base, kind: "hourly", minute };
  }
  if (fields.hours.length !== 1) return custom();
  const time = `${pad(fields.hours[0])}:${pad(minute)}`;
  if (allMonths && allDom) {
    if (fields.daysOfWeek.join() === "1,2,3,4,5") return { ...base, kind: "weekdays", time };
    if (fields.daysOfWeek.length === 1) return { ...base, kind: "weekly", time, weekday: fields.daysOfWeek[0] };
  }
  if (maxIterations === 1 && fields.months.length === 1 && fields.daysOfMonth.length === 1 && fields.daysOfWeek.length === 7) {
    const year = new Date().getFullYear();
    return { ...base, kind: "once", time, date: `${year}-${pad(fields.months[0])}-${pad(fields.daysOfMonth[0])}` };
  }
  return custom();
}

const UNIT_SECONDS: Record<string, number> = { s: 1, m: 60, h: 3600, d: 86_400 };

/** The stored spec (`every:1800` / `daily:09:00` / `cron:<expr>`) a request body will produce, so
 * the form can preview a cadence with the same label code the saved row uses. */
export function bodyToStored(body: CadenceBody["body"]): string | null {
  if (body.at) return `daily:${body.at}`;
  if (body.cron) return `cron:${body.cron}`;
  if (body.every) {
    const match = /^(\d+)([smhd])$/.exec(body.every.trim());
    return match ? `every:${parseInt(match[1], 10) * UNIT_SECONDS[match[2]]}` : null;
  }
  return null;
}
