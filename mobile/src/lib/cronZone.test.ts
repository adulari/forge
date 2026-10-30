import { describe, expect, it } from "vitest";

import { describeCron, describeStoredSpec } from "./cron";
import { formatInstant, isKnownTimeZone, nextCronRunInZone } from "./cronZone";

describe("describeCron", () => {
  it("says common shapes in words", () => {
    expect(describeCron("0 3 * * 1-5")).toBe("Weekdays at 3:00 AM");
    expect(describeCron("30 14 * * *")).toBe("Daily at 2:30 PM");
    expect(describeCron("0 9 * * 1")).toBe("Mondays at 9:00 AM");
    expect(describeCron("0 9 * * 0,6")).toBe("Weekends at 9:00 AM");
    expect(describeCron("0 9 * * 1,3,5")).toBe("Mon, Wed, Fri at 9:00 AM");
    expect(describeCron("0 * * * *")).toBe("Every hour");
    expect(describeCron("15 * * * *")).toBe("Every hour at :15");
    expect(describeCron("*/15 * * * *")).toBe("Every 15 minutes");
    expect(describeCron("0 0 1 * *")).toBe("Monthly on the 1st at 12:00 AM");
    expect(describeCron("0 8 30 7 *")).toBe("Jul 30 at 8:00 AM");
  });

  it("returns null for shapes it would only paraphrase", () => {
    expect(describeCron("0,30 9-17 * * 1-5")).toBeNull();
    expect(describeCron("not cron")).toBeNull();
  });
});

describe("describeStoredSpec", () => {
  it("covers every stored form and falls back to the daemon's label", () => {
    expect(describeStoredSpec("every:1800", "x")).toBe("Every 30 minutes");
    expect(describeStoredSpec("every:86400", "x")).toBe("Every day");
    expect(describeStoredSpec("daily:09:05", "x")).toBe("Daily at 9:05 AM");
    expect(describeStoredSpec("cron:0 3 * * 1-5", "x")).toBe("Weekdays at 3:00 AM");
    expect(describeStoredSpec("cron:Mon *-*-* 09:00:00", "OnCalendar `Mon`")).toBe("OnCalendar `Mon`");
  });
});

describe("nextCronRunInZone", () => {
  // 2026-07-29 12:00 UTC is a Wednesday.
  const now = Date.UTC(2026, 6, 29, 12, 0);

  it("reads the expression on the named zone's clock", () => {
    // 03:00 in Tokyo (UTC+9, no DST) is 18:00 UTC the previous day -> Thursday 03:00 JST.
    const next = nextCronRunInZone("0 3 * * 1-5", now, "Asia/Tokyo");
    expect(next).toBe(Date.UTC(2026, 6, 29, 18, 0));
  });

  it("skips weekends in that zone", () => {
    // Friday 2026-07-31 18:00 UTC is already Saturday in Tokyo, so the next weekday is Monday.
    const friday = Date.UTC(2026, 6, 31, 18, 30);
    expect(nextCronRunInZone("0 3 * * 1-5", friday, "Asia/Tokyo")).toBe(Date.UTC(2026, 7, 2, 18, 0));
  });

  it("follows daylight saving", () => {
    // New York is UTC-4 in July and UTC-5 in January.
    expect(nextCronRunInZone("0 9 * * *", now, "America/New_York")).toBe(Date.UTC(2026, 6, 29, 13, 0));
    const winter = Date.UTC(2026, 0, 15, 0, 0);
    expect(nextCronRunInZone("0 9 * * *", winter, "America/New_York")).toBe(Date.UTC(2026, 0, 15, 14, 0));
  });

  it("is strictly after now and null for unknown zones or bad expressions", () => {
    const exact = Date.UTC(2026, 6, 29, 3, 0);
    expect(nextCronRunInZone("0 3 * * *", exact, "UTC")).toBe(Date.UTC(2026, 6, 30, 3, 0));
    expect(nextCronRunInZone("0 3 * * *", now, "Mars/Olympus")).toBeNull();
    expect(nextCronRunInZone("nonsense", now, "UTC")).toBeNull();
  });
});

describe("zone helpers", () => {
  it("validates names and formats without the narrow no-break space", () => {
    expect(isKnownTimeZone("Europe/Amsterdam")).toBe(true);
    expect(isKnownTimeZone("Nope/Nowhere")).toBe(false);
    expect(formatInstant(Date.UTC(2026, 6, 30, 3, 0), "UTC")).toBe("Jul 30, 3:00 AM");
  });
});
