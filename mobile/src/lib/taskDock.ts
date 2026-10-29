import type { SnapshotTask } from "./ws";

export interface TaskDockSummary {
  done: number;
  total: number;
  /** The task the header names while the dock is folded. */
  headline: SnapshotTask | null;
  allDone: boolean;
}

export function summarizeTasks(tasks: readonly SnapshotTask[]): TaskDockSummary {
  const done = tasks.filter((task) => task.status === "done").length;
  const headline =
    tasks.find((task) => task.status === "in_progress") ??
    tasks.find((task) => task.status === "pending") ??
    tasks[tasks.length - 1] ??
    null;
  return { done, total: tasks.length, headline, allDone: tasks.length > 0 && done === tasks.length };
}

export interface TaskWindow {
  rows: { index: number; task: SnapshotTask }[];
  hiddenAbove: number;
  hiddenBelow: number;
}

/** At most `max` rows, kept around the task being worked on so a long plan never pushes the
 * composer off the screen. */
export function windowTasks(tasks: readonly SnapshotTask[], max: number): TaskWindow {
  if (tasks.length <= max) {
    return { rows: tasks.map((task, index) => ({ index, task })), hiddenAbove: 0, hiddenBelow: 0 };
  }
  const active = tasks.findIndex((task) => task.status === "in_progress");
  const anchor = active >= 0 ? active : Math.max(0, tasks.findIndex((task) => task.status === "pending"));
  const start = Math.max(0, Math.min(anchor - 1, tasks.length - max));
  const rows = tasks.slice(start, start + max).map((task, i) => ({ index: start + i, task }));
  return { rows, hiddenAbove: start, hiddenBelow: tasks.length - start - rows.length };
}
