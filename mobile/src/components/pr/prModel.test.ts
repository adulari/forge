import { describe, expect, it } from "vitest";

import type { PrChecks, PrSummary } from "../../lib/apiPr";
import { checksLabel, mergeGate, orderWorkflows, parsePinned, prChip, projectReps, runTone, shouldPoll, togglePinned } from "./prModel";

const checks = (passing: number, failing: number, pending: number): PrChecks => ({
  total: passing + failing + pending, passing, failing, pending, items: [],
});
const pr = (over: Partial<PrSummary> = {}): PrSummary => ({
  number: 7, url: "u", title: "T", state: "open", draft: false, head_ref: "f", base_ref: "main",
  head_sha: "abcdef1", mergeable: "mergeable", merge_state: "clean", review_decision: "",
  additions: 1, deletions: 1, changed_files: 1, checks: checks(2, 0, 0), ...over,
});

describe("pr chip", () => {
  it("reflects check state", () => {
    expect(prChip(pr()).tone).toBe("success");
    expect(prChip(pr({ checks: checks(1, 1, 0) })).tone).toBe("danger");
    expect(prChip(pr({ checks: checks(1, 0, 1) })).tone).toBe("warn");
    expect(prChip(pr({ checks: checks(0, 0, 0) })).tone).toBe("neutral");
  });
  it("shows terminal states instead of checks", () => {
    expect(prChip(pr({ state: "merged" })).label).toBe("#7 merged");
    expect(prChip(pr({ draft: true })).label).toBe("#7 draft");
  });
  it("words check counts", () => {
    expect(checksLabel(checks(2, 1, 0))).toBe("1 of 3 failing");
    expect(checksLabel(checks(2, 0, 1))).toBe("1 of 3 running");
    expect(checksLabel(checks(0, 0, 0))).toBe("no checks");
  });
  it("polls only while checks run on an open PR", () => {
    expect(shouldPoll(pr({ checks: checks(1, 0, 1) }))).toBe(true);
    expect(shouldPoll(pr())).toBe(false);
    expect(shouldPoll(pr({ state: "merged", checks: checks(0, 0, 1) }))).toBe(false);
    expect(shouldPoll(null)).toBe(false);
  });
});

describe("merge gate", () => {
  it("blocks drafts, conflicts and closed PRs but only warns on failing checks", () => {
    expect(mergeGate(pr()).allowed).toBe(true);
    expect(mergeGate(pr({ draft: true })).allowed).toBe(false);
    expect(mergeGate(pr({ mergeable: "conflicting" })).allowed).toBe(false);
    expect(mergeGate(pr({ state: "closed" })).reason).toBe("Already closed");
    const warned = mergeGate(pr({ checks: checks(1, 1, 0) }));
    expect(warned).toEqual({ allowed: true, reason: "Some checks are failing" });
  });
});

describe("workflows", () => {
  it("orders pinned first and keeps the rest stable", () => {
    const list = [{ path: "a" }, { path: "b" }, { path: "c" }];
    expect(orderWorkflows(list, ["c", "a"]).map((w) => w.path)).toEqual(["c", "a", "b"]);
    expect(orderWorkflows(list, []).map((w) => w.path)).toEqual(["a", "b", "c"]);
  });
  it("toggles and parses pins defensively", () => {
    expect(togglePinned(["a"], "b")).toEqual(["a", "b"]);
    expect(togglePinned(["a", "b"], "a")).toEqual(["b"]);
    expect(parsePinned("not json")).toEqual([]);
    expect(parsePinned('["x", 3]')).toEqual(["x"]);
    expect(parsePinned(null)).toEqual([]);
  });
  it("tones runs", () => {
    expect(runTone({ status: "in_progress", conclusion: "" })).toBe("warn");
    expect(runTone({ status: "completed", conclusion: "success" })).toBe("success");
    expect(runTone({ status: "completed", conclusion: "failure" })).toBe("danger");
    expect(runTone({ status: "completed", conclusion: "cancelled" })).toBe("neutral");
  });
});

describe("projectReps", () => {
  it("keeps the most recent session per directory", () => {
    const reps = projectReps([
      { id: "a", cwd: "/p/x", last_activity: 1 },
      { id: "b", cwd: "/p/x", last_activity: 5 },
      { id: "c", cwd: "/p/y", last_activity: 3 },
      { id: "d", cwd: "", last_activity: 9 },
    ]);
    expect(reps).toEqual([{ cwd: "/p/x", sessionId: "b" }, { cwd: "/p/y", sessionId: "c" }]);
  });
});
