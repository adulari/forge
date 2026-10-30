// "Next run" in an explicit IANA time zone. lib/cron.ts answers in the viewer's local zone; a
// schedule pinned to another zone (systemd reads `OnCalendar=... Asia/Tokyo`) needs the wall clock
// of THAT zone, which Intl can give us without a time-zone database of our own.
import { parseCron } from "./cron";

interface Wall {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
}

const formatters = new Map<string, Intl.DateTimeFormat>();

function formatter(timeZone: string): Intl.DateTimeFormat {
  let known = formatters.get(timeZone);
  if (!known) {
    known = new Intl.DateTimeFormat("en-US", {
      timeZone,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
    });
    formatters.set(timeZone, known);
  }
  return known;
}

/** True when the runtime knows this zone name. Hermes ships a trimmed Intl, so callers ask first. */
export function isKnownTimeZone(timeZone: string): boolean {
  try {
    formatter(timeZone).format(0);
    return true;
  } catch {
    return false;
  }
}

function wallClock(ms: number, timeZone: string): Wall {
  const parts: Record<string, number> = {};
  for (const part of formatter(timeZone).formatToParts(ms)) {
    if (part.type !== "literal") parts[part.type] = parseInt(part.value, 10);
  }
  return { year: parts.year, month: parts.month, day: parts.day, hour: parts.hour % 24, minute: parts.minute };
}

function wallAsUtc(wall: Wall): number {
  return Date.UTC(wall.year, wall.month - 1, wall.day, wall.hour, wall.minute);
}

/** The instant at which `timeZone`'s clock reads `wall`, or null when that wall time does not exist
 * (a spring-forward gap). A repeated fall-back time resolves to its first occurrence. */
function instantOf(wall: Wall, timeZone: string): number | null {
  const guess = wallAsUtc(wall);
  const firstOffset = wallAsUtc(wallClock(guess, timeZone)) - guess;
  let instant = guess - firstOffset;
  const check = wallClock(instant, timeZone);
  if (wallAsUtc(check) !== guess) {
    const secondOffset = wallAsUtc(wallClock(instant, timeZone)) - instant;
    instant = guess - secondOffset;
    if (wallAsUtc(wallClock(instant, timeZone)) !== guess) return null;
  }
  return instant;
}

const MAX_SEARCH_DAYS = 1461;

/** Next instant strictly after `now` that `expression` fires on `timeZone`'s calendar, or null when
 * the expression or zone is not understood (or never fires within four years). */
export function nextCronRunInZone(expression: string, now: number, timeZone: string): number | null {
  const fields = parseCron(expression);
  if (!fields || !isKnownTimeZone(timeZone)) return null;
  const start = wallClock(now, timeZone);

  for (let offset = 0; offset <= MAX_SEARCH_DAYS; offset += 1) {
    const day = new Date(Date.UTC(start.year, start.month - 1, start.day + offset));
    const month = day.getUTCMonth() + 1;
    const dom = fields.daysOfMonth.includes(day.getUTCDate());
    const dow = fields.daysOfWeek.includes(day.getUTCDay());
    const dayMatches = fields.domStar || fields.dowStar ? dom && dow : dom || dow;
    if (!fields.months.includes(month) || !dayMatches) continue;
    for (const hour of fields.hours) {
      for (const minute of fields.minutes) {
        const instant = instantOf(
          { year: day.getUTCFullYear(), month, day: day.getUTCDate(), hour, minute },
          timeZone,
        );
        if (instant !== null && instant > now) return instant;
      }
    }
  }
  return null;
}

/** `Jul 30, 3:00 AM` in `timeZone` (or the viewer's zone when omitted), without Intl's narrow
 * no-break space before AM/PM. */
export function formatInstant(ms: number, timeZone?: string): string {
  try {
    return new Intl.DateTimeFormat("en-US", {
      ...(timeZone ? { timeZone } : {}),
      month: "short",
      day: "numeric",
      hour: "numeric",
      minute: "2-digit",
    })
      .format(ms)
      .replace(/[\u202f\u00a0]/g, " ");
  } catch {
    return new Date(ms).toLocaleString();
  }
}

/** The zone names Intl offers, for the picker; empty where the runtime cannot list them. */
export function listTimeZones(): string[] {
  const intl = Intl as unknown as { supportedValuesOf?: (key: string) => string[] };
  if (typeof intl.supportedValuesOf !== "function") return [];
  try {
    return intl.supportedValuesOf("timeZone");
  } catch {
    return [];
  }
}

export function localTimeZone(): string | null {
  try {
    return new Intl.DateTimeFormat().resolvedOptions().timeZone ?? null;
  } catch {
    return null;
  }
}
