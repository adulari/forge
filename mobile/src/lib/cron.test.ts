import { describe, expect, it } from "vitest";

import { nextCronRun, parseCron } from "./cron";

// Every expectation is built with the local-time `Date` constructor, matching the module's
// documented local-time contract, so the suite is timezone-independent.
const at = (
  year: number,
  month: number,
  day: number,
  hour = 0,
  minute = 0,
): number => new Date(year, month - 1, day, hour, minute, 0, 0).getTime();

const next = (expression: string, now: number) => nextCronRun(expression, now);

describe("parseCron", () => {
  it("expands each field form into the values it matches", () => {
    expect(parseCron("*/15 9-17 1,15 * *")).toMatchObject({
      minutes: [0, 15, 30, 45],
      hours: [9, 10, 11, 12, 13, 14, 15, 16, 17],
      daysOfMonth: [1, 15],
      months: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
      daysOfWeek: [0, 1, 2, 3, 4, 5, 6],
    });
  });

  it("supports stepped ranges, open-ended steps and lists of ranges", () => {
    expect(parseCron("0-30/10 * * * *")?.minutes).toEqual([0, 10, 20, 30]);
    expect(parseCron("45/5 * * * *")?.minutes).toEqual([45, 50, 55]);
    expect(parseCron("0 1-3,20-21 * * *")?.hours).toEqual([1, 2, 3, 20, 21]);
  });

  it("treats 0 and 7 as the same Sunday and accepts three-letter names", () => {
    expect(parseCron("0 0 * * 7")?.daysOfWeek).toEqual([0]);
    expect(parseCron("0 0 * * 0,7")?.daysOfWeek).toEqual([0]);
    expect(parseCron("0 0 * * SUN")?.daysOfWeek).toEqual([0]);
    expect(parseCron("0 0 * * mon-fri")?.daysOfWeek).toEqual([1, 2, 3, 4, 5]);
    expect(parseCron("0 0 * JAN,DEC *")?.months).toEqual([1, 12]);
  });

  it("records Vixie's star flags from the first character of the day fields", () => {
    expect(parseCron("0 0 * * 1")).toMatchObject({ domStar: true, dowStar: false });
    expect(parseCron("0 0 15 * *")).toMatchObject({ domStar: false, dowStar: true });
    // `*/2` begins with `*`, so reference cron still counts the field as starred.
    expect(parseCron("0 0 */2 * 1")).toMatchObject({ domStar: true, dowStar: false });
  });

  it("expands the @ macros and rejects @reboot", () => {
    expect(parseCron("@daily")).toMatchObject({ minutes: [0], hours: [0] });
    expect(parseCron("@weekly")).toMatchObject({ daysOfWeek: [0] });
    expect(parseCron("@yearly")).toMatchObject({ months: [1], daysOfMonth: [1] });
    expect(parseCron("@reboot")).toBeNull();
  });

  it.each([
    ["too few fields", "0 0 * *"],
    ["too many fields", "0 0 * * * *"],
    ["empty", "   "],
    ["minute out of range", "60 0 * * *"],
    ["hour out of range", "0 24 * * *"],
    ["day-of-month out of range", "0 0 32 * *"],
    ["month out of range", "0 0 * 13 *"],
    ["day-of-week out of range", "0 0 * * 8"],
    ["reversed range", "0 0 * * FRI-MON"],
    ["zero step", "*/0 * * * *"],
    ["non-numeric step", "*/x * * * *"],
    ["double step", "0/2/3 * * * *"],
    ["triple range", "1-2-3 * * * *"],
    ["empty list term", "1,,2 * * * *"],
    ["unknown name", "0 0 * * FUNDAY"],
    ["systemd OnCalendar", "Mon *-*-* 09:00:00"],
    ["free text", "every monday at nine"],
  ])("returns null for an expression it cannot parse: %s", (_label, expression) => {
    expect(parseCron(expression)).toBeNull();
    expect(nextCronRun(expression, at(2026, 7, 25, 12, 0))).toBeNull();
  });
});

describe("nextCronRun", () => {
  it("never returns the current minute, only the next matching one", () => {
    const now = at(2026, 7, 25, 12, 0);
    expect(next("* * * * *", now)).toBe(at(2026, 7, 25, 12, 1));
    expect(next("0 12 * * *", now)).toBe(at(2026, 7, 26, 12, 0));
  });

  it("ignores seconds already elapsed inside the current minute", () => {
    const now = new Date(2026, 6, 25, 12, 0, 31, 500).getTime();
    expect(next("* * * * *", now)).toBe(at(2026, 7, 25, 12, 1));
  });

  it("finds the next slot within the day for stepped and listed fields", () => {
    const now = at(2026, 7, 25, 9, 7);
    expect(next("*/15 * * * *", now)).toBe(at(2026, 7, 25, 9, 15));
    expect(next("0 9-17 * * *", now)).toBe(at(2026, 7, 25, 10, 0));
    expect(next("30 6,18 * * *", now)).toBe(at(2026, 7, 25, 18, 30));
  });

  it("rolls over into the next month when the day has passed", () => {
    // 2026-01-31 is past the 15th, so the next day-of-month hit is in February.
    expect(next("0 2 15 * *", at(2026, 1, 31, 23, 59))).toBe(at(2026, 2, 15, 2, 0));
    // 2026-12-31 rolls the year as well.
    expect(next("0 0 1 * *", at(2026, 12, 31, 12, 0))).toBe(at(2027, 1, 1, 0, 0));
  });

  it("skips months that are too short for the requested day", () => {
    // February 2026 has 28 days: the 31st jumps straight to March.
    expect(next("0 0 31 * *", at(2026, 1, 31, 12, 0))).toBe(at(2026, 3, 31, 0, 0));
    // April has 30 days, so a 31st in April lands in May.
    expect(next("0 0 31 * *", at(2026, 4, 1, 0, 0))).toBe(at(2026, 5, 31, 0, 0));
  });

  it("handles leap years for Feb 29", () => {
    expect(next("0 0 29 2 *", at(2024, 2, 1, 0, 0))).toBe(at(2024, 2, 29, 0, 0));
    // 2026 is not a leap year, so the next Feb 29 is 2028.
    expect(next("0 0 29 2 *", at(2026, 3, 1, 0, 0))).toBe(at(2028, 2, 29, 0, 0));
    // 2100 is a century year not divisible by 400, so February 29 skips it — the next one is
    // 2104, just inside the search bound.
    expect(next("0 0 29 2 *", at(2100, 3, 1, 0, 0))).toBe(at(2104, 2, 29, 0, 0));
  });

  describe("day-of-month / day-of-week", () => {
    it("ANDs when one of the two fields is a star", () => {
      // 2026-07-25 is a Saturday; the next Monday is the 27th.
      expect(next("0 6 * * 1", at(2026, 7, 25, 12, 0))).toBe(at(2026, 7, 27, 6, 0));
      expect(next("0 6 27 * *", at(2026, 7, 25, 12, 0))).toBe(at(2026, 7, 27, 6, 0));
    });

    it("ORs when BOTH fields are restricted — the classic cron rule", () => {
      // `0 0 1 * 1` = the 1st of the month OR any Monday, whichever comes first.
      // From Sat 2026-07-25 the next Monday (the 27th) beats the 1st of August.
      expect(next("0 0 1 * 1", at(2026, 7, 25, 12, 0))).toBe(at(2026, 7, 27, 0, 0));
      // From Mon 2026-07-27 12:00 the day-of-month half wins: Sat 2026-08-01 lands before the
      // following Monday (the 3rd). Under AND semantics neither would fire until 2027-02-01.
      expect(next("0 0 1 * 1", at(2026, 7, 27, 12, 0))).toBe(at(2026, 8, 1, 0, 0));
    });

    it("keeps ANDing when the day-of-month field merely starts with a star", () => {
      // Vixie: `*/2` sets DOM_STAR, so this is "every 2nd day AND Monday", not an OR.
      // From Sat 2026-07-25 the Mondays are the 27th (odd) and Aug 3rd (odd) — `*/2` covers the
      // odd days 1,3,...,27, so the 27th matches both halves.
      expect(next("0 0 */2 * 1", at(2026, 7, 25, 12, 0))).toBe(at(2026, 7, 27, 0, 0));
      // The same stepped set written without the leading star is NOT starred, so it ORs instead:
      // the next even day (Sun the 26th) fires even though it is not a Monday.
      expect(next("0 0 2-30/2 * 1", at(2026, 7, 25, 12, 0))).toBe(at(2026, 7, 26, 0, 0));
    });
  });

  it("gives up rather than hanging on an expression that never fires", () => {
    // February never has a 30th, and day-of-week is a star so there is no OR escape hatch.
    expect(next("0 0 30 2 *", at(2026, 7, 25, 12, 0))).toBeNull();
    // Real but beyond the ~4-year search bound: after 2096 the next Feb 29 is 2104 (2100 is not a
    // leap year). Null is the honest answer — the caller falls back to "last run".
    expect(next("0 0 29 2 *", at(2096, 3, 1, 0, 0))).toBeNull();
  });

  it("terminates quickly on the unsatisfiable case", () => {
    const started = Date.now();
    expect(next("0 0 30 2 *", at(2026, 7, 25, 12, 0))).toBeNull();
    expect(Date.now() - started).toBeLessThan(1000);
  });

  it("defaults now to the current clock and stays in the future", () => {
    const result = nextCronRun("* * * * *");
    expect(result).not.toBeNull();
    expect(result as number).toBeGreaterThan(Date.now());
  });
});
