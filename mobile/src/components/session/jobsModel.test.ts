import { describe, expect, it } from "vitest";

import { jobAge, orderJobs } from "./jobsModel";

const job = (id: number, running: boolean, started_at: number) => ({
  id, command: "c", running, status: running ? "running" : "exited 0", started_at,
});

describe("jobsModel", () => {
  it("lists running jobs first, newest first within a group", () => {
    const ordered = orderJobs([job(1, false, 30), job(2, true, 10), job(3, true, 20), job(4, false, 40)]);
    expect(ordered.map((j) => j.id)).toEqual([3, 2, 4, 1]);
  });
  it("renders coarse ages", () => {
    expect(jobAge(100, 130)).toBe("30s");
    expect(jobAge(0, 600)).toBe("10m");
    expect(jobAge(0, 7300)).toBe("2h");
    expect(jobAge(0, 200000)).toBe("2d");
  });
});
