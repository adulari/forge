// Standard 5-field cron parser + next-occurrence search, dependency-free. The Schedules panel
// stores a spec verbatim (`every:1800` / `daily:09:00` / `cron:<expr>`); the first two are trivial
// arithmetic, the third needs this so a cron row can show a real "next 02:00" instead of falling
// back to "last run".
//
// TIMEZONE — every computation here is in the *local* timezone of the device doing the rendering,
// never UTC. That is deliberate: a Forge schedule fires from a native OS timer on the daemon's host
// (systemd `OnCalendar=`, launchd `StartCalendarInterval`, Windows `schtasks`), and all three read a
// calendar spec as host-local wall-clock time. The desktop shell normally runs on that same host, so
// local time is the honest answer. Viewing a remote host from another timezone is the known gap: the
// wire's `ScheduleRow` carries no host UTC offset, so there is nothing to correct with, and the
// label is the viewer's wall clock. Nothing here mixes the two — no `Date.UTC`, no `getUTC*`.
//
// DST follows from that: the search walks local calendar days, so on a spring-forward day a spec
// pointing into the skipped hour lands on the next real instant (JS `Date` normalises it), and on a
// fall-back day the first of the two repeated wall-clock times wins. Both match what an OS timer
// does closely enough for a label; neither can silently drift a whole day.

/** The set of instants a parsed expression matches, per field. `*Star` mirrors Vixie cron's
 * DOM_STAR/DOW_STAR flags, which decide whether day-of-month and day-of-week AND or OR. */
export interface CronFields {
  minutes: number[];
  hours: number[];
  daysOfMonth: number[];
  months: number[];
  /** Normalised to 0-6; cron's 7 (Sunday) is folded onto 0. */
  daysOfWeek: number[];
  domStar: boolean;
  dowStar: boolean;
}

const MONTH_NAMES: Record<string, number> = {
  JAN: 1, FEB: 2, MAR: 3, APR: 4, MAY: 5, JUN: 6,
  JUL: 7, AUG: 8, SEP: 9, OCT: 10, NOV: 11, DEC: 12,
};

const DOW_NAMES: Record<string, number> = {
  SUN: 0, MON: 1, TUE: 2, WED: 3, THU: 4, FRI: 5, SAT: 6,
};

/** The `@`-macros every cron implementation accepts, expanded to their documented equivalents.
 * `@reboot` has no calendar meaning and stays unsupported (parses to null). */
const MACROS: Record<string, string> = {
  "@yearly": "0 0 1 1 *",
  "@annually": "0 0 1 1 *",
  "@monthly": "0 0 1 * *",
  "@weekly": "0 0 * * 0",
  "@daily": "0 0 * * *",
  "@midnight": "0 0 * * *",
  "@hourly": "0 * * * *",
};

/** ~4 years of calendar days. Bounds the search so an expression that can never fire (`0 0 30 2 *`)
 * terminates with null instead of spinning, at the cost of also giving up on the handful of specs
 * whose gap genuinely exceeds four years (Feb 29 across a skipped century leap year). Returning null
 * is the safe direction: the caller degrades to "last run" rather than showing a wrong time. */
const MAX_SEARCH_DAYS = 1461;

function parseValue(raw: string, names?: Record<string, number>): number | null {
  if (/^\d+$/.test(raw)) return parseInt(raw, 10);
  const named = names?.[raw.toUpperCase()];
  return named ?? null;
}

/** One comma-separated field into the explicit list of values it matches, or null if any term is
 * malformed. Strict on purpose — a field we do not fully understand must not produce a time. */
function parseField(
  field: string,
  min: number,
  max: number,
  names?: Record<string, number>,
): number[] | null {
  const hit = new Set<number>();
  for (const term of field.split(",")) {
    const parts = term.split("/");
    if (parts.length > 2) return null;
    const [spec, stepRaw] = parts;

    let step = 1;
    if (stepRaw !== undefined) {
      if (!/^\d+$/.test(stepRaw)) return null;
      step = parseInt(stepRaw, 10);
      if (step === 0) return null;
    }

    let from: number;
    let to: number;
    if (spec === "*") {
      from = min;
      to = max;
    } else {
      const bounds = spec.split("-");
      if (bounds.length > 2) return null;
      const start = parseValue(bounds[0], names);
      if (start === null) return null;
      if (bounds.length === 2) {
        const end = parseValue(bounds[1], names);
        if (end === null) return null;
        from = start;
        to = end;
      } else {
        // A bare `a` is itself; `a/n` means "from a to the end of the field", which is how every
        // widely used cron reads it. Reversed ranges (`FRI-MON`) are an error in Vixie cron rather
        // than a wrap, so they fall through to the bounds check below and return null.
        from = start;
        to = stepRaw === undefined ? start : max;
      }
    }
    if (from < min || to > max || from > to) return null;
    for (let value = from; value <= to; value += step) hit.add(value);
  }
  return [...hit].sort((a, b) => a - b);
}

/** Parse a 5-field expression (`minute hour day-of-month month day-of-week`) or an `@`-macro.
 * Returns null for anything not fully understood — never a partial guess. */
export function parseCron(expression: string): CronFields | null {
  const trimmed = expression.trim();
  if (!trimmed) return null;
  const expanded = trimmed.startsWith("@") ? MACROS[trimmed.toLowerCase()] : trimmed;
  if (!expanded) return null;

  const fields = expanded.split(/\s+/);
  if (fields.length !== 5) return null;
  const [minute, hour, dom, month, dow] = fields;

  const minutes = parseField(minute, 0, 59);
  const hours = parseField(hour, 0, 23);
  const daysOfMonth = parseField(dom, 1, 31);
  const months = parseField(month, 1, 12, MONTH_NAMES);
  // 0 and 7 are both Sunday, so the field is parsed over 0-7 and folded onto 0-6 afterwards.
  const rawDow = parseField(dow, 0, 7, DOW_NAMES);
  if (!minutes || !hours || !daysOfMonth || !months || !rawDow) return null;
  const daysOfWeek = [...new Set(rawDow.map((day) => day % 7))].sort((a, b) => a - b);

  return {
    minutes,
    hours,
    daysOfMonth,
    months,
    daysOfWeek,
    // Vixie sets the star flag from the field's *first character*, so `*/2` counts as starred and
    // still ANDs. Mirrored rather than "fixed" — this is the reference implementation's behaviour.
    domStar: dom.startsWith("*"),
    dowStar: dow.startsWith("*"),
  };
}

/**
 * The classic cron rule: when day-of-month and day-of-week are BOTH restricted, a match on either
 * one fires (OR). When at least one of them is `*`, both must match (AND). Getting this backwards
 * is the canonical cron bug, so it lives in one place.
 */
function dayMatches(fields: CronFields, date: Date): boolean {
  if (!fields.months.includes(date.getMonth() + 1)) return false;
  // Month lengths and leap years need no special case: `getDate()` never yields 31 in a 30-day
  // month or 29 in a common-year February, so those days simply never match.
  const dom = fields.daysOfMonth.includes(date.getDate());
  const dow = fields.daysOfWeek.includes(date.getDay());
  return fields.domStar || fields.dowStar ? dom && dow : dom || dow;
}

/**
 * Next instant strictly after `now` that the expression fires, in local time, or null when the
 * expression cannot be parsed or does not fire within {@link MAX_SEARCH_DAYS}.
 *
 * `now` is injected (not read from the module) so callers stay pure under `react-hooks/purity` and
 * so tests are deterministic.
 */
export function nextCronRun(expression: string, now: number = Date.now()): number | null {
  const fields = parseCron(expression);
  if (!fields) return null;

  const hours = new Set(fields.hours);
  const minutes = new Set(fields.minutes);

  // Start one whole minute ahead: cron fires on minute boundaries, and "next" must be in the future
  // even when this very minute matches.
  const cursor = new Date(now);
  cursor.setSeconds(0, 0);
  cursor.setMinutes(cursor.getMinutes() + 1);

  // Walk whole local days rather than minutes — 1461 iterations instead of ~2.1 million.
  for (let day = 0; day <= MAX_SEARCH_DAYS; day += 1) {
    if (dayMatches(fields, cursor)) {
      const fromHour = cursor.getHours();
      for (let hour = fromHour; hour < 24; hour += 1) {
        if (!hours.has(hour)) continue;
        const fromMinute = hour === fromHour ? cursor.getMinutes() : 0;
        for (let minute = fromMinute; minute < 60; minute += 1) {
          if (!minutes.has(minute)) continue;
          const hit = new Date(cursor);
          hit.setHours(hour, minute, 0, 0);
          // A repeated wall-clock hour (DST fall-back) can resolve behind `now`; keep scanning
          // rather than hand the caller a "next run" in the past.
          if (hit.getTime() > now) return hit.getTime();
        }
      }
    }
    cursor.setDate(cursor.getDate() + 1);
    cursor.setHours(0, 0, 0, 0);
  }
  return null;
}
