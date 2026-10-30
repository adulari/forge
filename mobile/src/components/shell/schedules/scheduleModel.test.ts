import { describe, expect, it } from "vitest";

import type { ScheduleRow, ScheduleRun } from "../../../lib/api";
import { cadenceLabel, filterSchedules, lastResultLine, nextRunLabel, nextRunMs } from "./scheduleModel";

const row = (change: Partial<ScheduleRow> = {}): ScheduleRow => ({
  id: "s1",
  task: "Audit dependencies",
  cwd: "/work/app",
  mode: null,
  model: null,
  cron: "cron:0 3 * * 1-5",
  spec_label: "cron `0 3 * * 1-5`",
  enabled: true,
  created_at: 1_700_000_000,
  last_run: null,
  tz: null,
  max_runtime_secs: null,
  max_iterations: null,
  retries: 0,
  backoff_secs: 60,
  misfire: "run",
  stop_on_error: false,
  stop_when: null,
  latest_run: null,
  ...change,
});

const run = (change: Partial<ScheduleRun>): ScheduleRun => ({
  id: "r1",
  schedule_id: "s1",
  started_at: 1_000,
  finished_at: 1_060,
  status: "ok",
  trigger: "timer",
  attempt: 1,
  session_id: "sess",
  summary: null,
  error: null,
  read: false,
  ...change,
});

describe("next run", () => {
  const now = Date.UTC(2026, 6, 29, 12, 0);

  it("reads a zoned cron on that zone's clock", () => {
    expect(nextRunMs(row({ tz: "Asia/Tokyo" }), now)).toBe(Date.UTC(2026, 6, 29, 18, 0));
    expect(nextRunLabel(row({ tz: "Asia/Tokyo" }), now)).toBe("Next run Jul 30, 3:00 AM");
  });

  it("handles daily rows in a zone and interval rows from the last tick", () => {
    expect(nextRunMs(row({ cron: "daily:09:00", tz: "America/New_York" }), now)).toBe(Date.UTC(2026, 6, 29, 13, 0));
    const every = row({ cron: "every:3600", last_run: Math.floor(now / 1000) - 600 });
    expect(nextRunMs(every, now)).toBe(now - 600_000 + 3_600_000);
  });

  it("says Paused for a paused row and falls back for OnCalendar specs", () => {
    expect(nextRunLabel(row({ enabled: false }), now)).toBe("Paused");
    expect(nextRunMs(row({ cron: "cron:Mon *-*-* 09:00:00" }), now)).toBeNull();
    expect(nextRunLabel(row({ cron: "cron:Mon *-*-* 09:00:00" }), now)).toBe("Not run yet");
  });

  it("puts the zone in the cadence label", () => {
    expect(cadenceLabel(row({ tz: "Asia/Tokyo" }))).toBe("Weekdays at 3:00 AM (Asia/Tokyo)");
  });
});

describe("lastResultLine", () => {
  const now = 1_060_000 + 240_000;
  it("reports status, age and the useful detail", () => {
    expect(lastResultLine(null, now)).toBeNull();
    expect(lastResultLine(run({ summary: "All clear" }), now)).toBe("Succeeded 4m ago - All clear");
    expect(lastResultLine(run({ status: "failed", error: "stopped after hitting the 600s runtime limit" }), now)).toBe(
      "Failed 4m ago - stopped after hitting the 600s runtime limit",
    );
    expect(lastResultLine(run({ status: "interrupted", error: "the process stopped before the run finished" }), now)).toContain(
      "Interrupted",
    );
    expect(lastResultLine(run({ status: "running", finished_at: null }), now)).toMatch(/^Running since/);
  });
});

describe("filterSchedules", () => {
  const rows = [
    row({ id: "a", task: "Daily brief", enabled: true }),
    row({ id: "b", task: "Dependency audit", enabled: false, cwd: "/work/api" }),
  ];
  it("filters by state and by text across task, folder and cadence", () => {
    expect(filterSchedules(rows, "active", "").map((r) => r.id)).toEqual(["a"]);
    expect(filterSchedules(rows, "paused", "").map((r) => r.id)).toEqual(["b"]);
    expect(filterSchedules(rows, "all", "AUDIT").map((r) => r.id)).toEqual(["b"]);
    expect(filterSchedules(rows, "all", "/work/api").map((r) => r.id)).toEqual(["b"]);
    expect(filterSchedules(rows, "all", "weekdays").length).toBe(2);
    expect(filterSchedules(rows, "paused", "brief")).toEqual([]);
  });
});
