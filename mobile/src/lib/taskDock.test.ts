import { describe, expect, it } from "vitest";

import { summarizeTasks, windowTasks } from "./taskDock";
import type { SnapshotTask } from "./ws";

const t = (title: string, status: SnapshotTask["status"]): SnapshotTask => ({ title, status });

describe("summarizeTasks", () => {
  it("counts done tasks and names the one in progress", () => {
    const summary = summarizeTasks([t("a", "done"), t("b", "in_progress"), t("c", "pending")]);
    expect(summary).toMatchObject({ done: 1, total: 3, allDone: false });
    expect(summary.headline?.title).toBe("b");
  });

  it("names the next pending task when nothing is in progress", () => {
    expect(summarizeTasks([t("a", "done"), t("b", "pending")]).headline?.title).toBe("b");
  });

  it("falls back to the last task once everything is done, and to nothing for an empty list", () => {
    const done = summarizeTasks([t("a", "done"), t("b", "done")]);
    expect(done.allDone).toBe(true);
    expect(done.headline?.title).toBe("b");
    expect(summarizeTasks([])).toEqual({ done: 0, total: 0, headline: null, allDone: false });
  });
});

describe("windowTasks", () => {
  const plan = Array.from({ length: 12 }, (_, i) => t(`step ${i}`, i < 5 ? "done" : i === 5 ? "in_progress" : "pending"));

  it("shows a short plan whole", () => {
    expect(windowTasks(plan.slice(0, 4), 6)).toMatchObject({ hiddenAbove: 0, hiddenBelow: 0 });
  });

  it("keeps the active task in view with one finished task above it", () => {
    const window = windowTasks(plan, 6);
    expect(window.rows.map((row) => row.index)).toEqual([4, 5, 6, 7, 8, 9]);
    expect(window.hiddenAbove).toBe(4);
    expect(window.hiddenBelow).toBe(2);
  });

  it("stops at the end of the list instead of leaving blank space", () => {
    const tail = plan.map((task, i) => (i === 11 ? t(task.title, "in_progress") : t(task.title, "done")));
    const window = windowTasks(tail, 6);
    expect(window.rows.map((row) => row.index)).toEqual([6, 7, 8, 9, 10, 11]);
    expect(window.hiddenBelow).toBe(0);
  });

  it("follows the first pending task when none is in progress", () => {
    const idle = plan.map((task, i) => t(task.title, i < 8 ? "done" : "pending"));
    expect(windowTasks(idle, 6).rows.map((row) => row.index)).toEqual([6, 7, 8, 9, 10, 11]);
  });
});
