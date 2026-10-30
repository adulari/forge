import { describe, expect, it } from "vitest";

import { bodyToStored, cadenceFromStored, cadenceToBody, defaultCadence, type Cadence } from "./scheduleCadence";

const make = (change: Partial<Cadence>): Cadence => ({ ...defaultCadence(), ...change });

describe("cadenceToBody", () => {
  it("compiles each friendly shape to the daemon's one-of body", () => {
    expect(cadenceToBody(make({ kind: "hourly", minute: 15 }))?.body).toEqual({ cron: "15 * * * *" });
    expect(cadenceToBody(make({ kind: "daily", time: "9:05" }))?.body).toEqual({ at: "09:05" });
    expect(cadenceToBody(make({ kind: "weekdays", time: "03:00" }))?.body).toEqual({ cron: "0 3 * * 1-5" });
    expect(cadenceToBody(make({ kind: "weekly", time: "16:30", weekday: 5 }))?.body).toEqual({ cron: "30 16 * * 5" });
  });

  it("makes a one-time schedule a dated cron that stops after one run", () => {
    expect(cadenceToBody(make({ kind: "once", time: "08:00", date: "2026-07-30" }))).toEqual({
      body: { cron: "0 8 30 7 *" },
      maxIterations: 1,
    });
    expect(cadenceToBody(make({ kind: "once", time: "08:00", date: "2026-02-31" }))).toBeNull();
  });

  it("accepts the stored and bare custom forms and rejects junk", () => {
    expect(cadenceToBody(make({ kind: "custom", custom: "every:1800" }))?.body).toEqual({ every: "1800s" });
    expect(cadenceToBody(make({ kind: "custom", custom: "30m" }))?.body).toEqual({ every: "30m" });
    expect(cadenceToBody(make({ kind: "custom", custom: "daily:02:00" }))?.body).toEqual({ at: "02:00" });
    expect(cadenceToBody(make({ kind: "custom", custom: "0 6 * * 1" }))?.body).toEqual({ cron: "0 6 * * 1" });
    expect(cadenceToBody(make({ kind: "custom", custom: "Mon *-*-* 09:00:00" }))?.body).toEqual({
      cron: "Mon *-*-* 09:00:00",
    });
    expect(cadenceToBody(make({ kind: "custom", custom: "whenever" }))).toBeNull();
    expect(cadenceToBody(make({ kind: "daily", time: "25:00" }))).toBeNull();
    expect(cadenceToBody(make({ kind: "hourly", minute: 75 }))).toBeNull();
  });
});

describe("cadenceFromStored", () => {
  it("round-trips what the builder produces", () => {
    const cases: Cadence[] = [
      make({ kind: "hourly", minute: 15 }),
      make({ kind: "daily", time: "09:05" }),
      make({ kind: "weekdays", time: "03:00" }),
      make({ kind: "weekly", time: "16:30", weekday: 5 }),
    ];
    for (const cadence of cases) {
      const stored = bodyToStored(cadenceToBody(cadence)!.body)!;
      expect(cadenceFromStored(stored, null)).toEqual(cadence);
    }
  });

  it("keeps anything it cannot model as custom so editing never loses the spec", () => {
    expect(cadenceFromStored("every:1800", null)).toMatchObject({ kind: "custom", custom: "every:1800" });
    expect(cadenceFromStored("cron:0,30 9-17 * * 1-5", null)).toMatchObject({ kind: "custom" });
    expect(cadenceFromStored("cron:Mon *-*-* 09:00:00", null)).toMatchObject({ kind: "custom" });
  });

  it("recognises a one-time schedule by its single-run cap", () => {
    expect(cadenceFromStored("cron:0 8 30 7 *", 1)).toMatchObject({ kind: "once", time: "08:00" });
    expect(cadenceFromStored("cron:0 8 30 7 *", null).kind).toBe("custom");
  });
});
