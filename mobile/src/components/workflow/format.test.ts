import { describe, expect, it } from "vitest";

import type { SnapshotSubagent, SnapshotWorkflow } from "../../lib/ws";
import {
  extractJson,
  formatDuration,
  groupByPhase,
  isFailed,
  totalCost,
  workflowRows,
  workflowTitle,
} from "./format";

function sub(partial: Partial<SnapshotSubagent>): SnapshotSubagent {
  return {
    id: "a",
    agent: "general",
    task: "t",
    model: "claude-opus-4-8",
    phase: null,
    last: "",
    done: false,
    ok: true,
    cost: 0,
    ...partial,
  };
}

describe("workflowRows", () => {
  it("keeps only rows that carry a phase (workflow agents), dropping plain subagents", () => {
    const rows = [sub({ id: "1", phase: "Scan" }), sub({ id: "2", phase: null }), sub({ id: "3", phase: "Fix" })];
    expect(workflowRows(rows).map((r) => r.id)).toEqual(["1", "3"]);
  });
  it("returns empty for a batch with no workflow rows", () => {
    expect(workflowRows([sub({ phase: null })])).toEqual([]);
  });
});

describe("groupByPhase", () => {
  it("preserves the declared phase order and emits zero-row phases as pending", () => {
    const rows = [sub({ id: "1", phase: "Fix", done: true }), sub({ id: "2", phase: "Scan" })];
    const groups = groupByPhase(rows, ["Scan", "Fix", "Verify"]);
    expect(groups.map((g) => g.phase)).toEqual(["Scan", "Fix", "Verify"]);
    expect(groups[2]).toMatchObject({ phase: "Verify", state: "pending", rows: [] });
  });
  it("marks a phase running while any row is not done, done once all finish", () => {
    const running = groupByPhase([sub({ phase: "Scan", done: false }), sub({ phase: "Scan", done: true })], ["Scan"]);
    expect(running[0].state).toBe("running");
    const done = groupByPhase([sub({ phase: "Scan", done: true }), sub({ phase: "Scan", done: true })], ["Scan"]);
    expect(done[0]).toMatchObject({ state: "done", doneCount: 2, runningCount: 0 });
  });
  it("collects rows with an undeclared phase into a trailing 'other' group", () => {
    const groups = groupByPhase([sub({ phase: "Ghost" })], ["Scan"]);
    expect(groups.map((g) => g.phase)).toEqual(["Scan", "other"]);
    expect(groups[1].unknown).toBe(true);
  });
  it("sums per-phase cost", () => {
    const groups = groupByPhase([sub({ phase: "Scan", cost: 0.2 }), sub({ phase: "Scan", cost: 0.4 })], ["Scan"]);
    expect(groups[0].cost).toBeCloseTo(0.6);
  });
});

describe("totalCost / isFailed / workflowTitle", () => {
  it("totals row costs, tolerating missing cost", () => {
    expect(totalCost([sub({ cost: 0.1 }), sub({ cost: 0.25 })])).toBeCloseTo(0.35);
  });
  it("flags a finished-but-not-ok row as failed, never a running one", () => {
    expect(isFailed(sub({ done: true, ok: false }))).toBe(true);
    expect(isFailed(sub({ done: false, ok: false }))).toBe(false);
    expect(isFailed(sub({ done: true, ok: true }))).toBe(false);
  });
  it("falls back to 'workflow' when a run has no name", () => {
    expect(workflowTitle({ name: "release-prep" } as SnapshotWorkflow)).toBe("release-prep");
    expect(workflowTitle({ name: null } as SnapshotWorkflow)).toBe("workflow");
    expect(workflowTitle({ name: "" } as SnapshotWorkflow)).toBe("workflow");
  });
});

describe("formatDuration", () => {
  it("renders s / m s / h m with zero-padding at boundaries", () => {
    expect(formatDuration(45)).toBe("45s");
    expect(formatDuration(92)).toBe("1m 32s");
    expect(formatDuration(124)).toBe("2m 04s");
    expect(formatDuration(3840)).toBe("1h 04m");
    expect(formatDuration(-5)).toBe("0s");
  });
});

describe("extractJson", () => {
  it("parses a structured result object, including one embedded in prose", () => {
    expect(extractJson('{"fixed": true, "tests": 8}')).toEqual({ fixed: true, tests: 8 });
    expect(extractJson('result: {"version":"2.7.0"} done')).toEqual({ version: "2.7.0" });
  });
  it("returns null for plain prose or a bare string (not a structured block)", () => {
    expect(extractJson("patched token.rs, all green")).toBeNull();
    expect(extractJson('"just a quoted string"')).toBeNull();
    expect(extractJson(null)).toBeNull();
    expect(extractJson("")).toBeNull();
  });
});
