import { describe, expect, it } from "vitest";

import type { CiPlanResponse } from "./apiCi";
import { ciBanner, ciDuration, ciGateNote, ciHeadline, ciSections } from "./ciModel";

const plan = (over: Partial<CiPlanResponse> = {}): CiPlanResponse => ({
  rows: [
    { state: "pass", key: "ci.yml::lint::fmt", job: "lint", name: "lint / fmt", detail: "cargo fmt", secs: 3 },
    { state: "fail", key: "ci.yml::lint::clippy", job: "lint", name: "lint / clippy", detail: "cargo clippy", tail: "boom", secs: 9 },
    { state: "skip", name: "win", detail: "runs on windows-latest (not Linux)", secs: 0 },
  ],
  noops: 1,
  gate: "auto",
  ...over,
});

describe("ciSections", () => {
  it("separates runnable steps from skips and keeps each skip's reason", () => {
    const { steps, skips } = ciSections(plan());
    expect(steps.map((s) => s.name)).toEqual(["lint / fmt", "lint / clippy"]);
    expect(skips).toEqual([expect.objectContaining({ name: "win", detail: "runs on windows-latest (not Linux)" })]);
  });
  it("is empty before the plan loads", () => {
    expect(ciSections(undefined)).toEqual({ steps: [], skips: [] });
  });
});

describe("ciHeadline", () => {
  it("counts failures, celebrates a clean run and says when nothing can run", () => {
    expect(ciHeadline(plan(), false)).toBe("1 of 2 steps failed.");
    expect(ciHeadline(plan({ rows: [plan().rows[0]], ok: true }), false)).toBe("All 1 steps passed.");
    expect(ciHeadline(plan({ rows: [plan().rows[2]] }), false)).toContain("Nothing in the workflows can run here");
    expect(ciHeadline(plan({ rows: [] }), false)).toBe("No GitHub workflows found in this repository.");
    expect(ciHeadline(undefined, false)).toContain("Reading");
    expect(ciHeadline(plan(), true)).toContain("Running");
  });
});

describe("ciGateNote and ciDuration", () => {
  it("says whether goals and pushes wait on the plan", () => {
    expect(ciGateNote(plan())).toContain("only after this plan passes");
    expect(ciGateNote(plan({ gate: "off" }))).toContain("Gate is off");
    expect(ciGateNote(undefined)).toBeNull();
  });
  it("formats seconds", () => {
    expect([ciDuration(0), ciDuration(9), ciDuration(125)]).toEqual(["", "9s", "2m 5s"]);
  });
});

describe("ciBanner", () => {
  const base = { runnable: 3, skipped: 1, steps: [] };
  it("names the step that blocked a goal or a push", () => {
    const goal = ciBanner({ ...base, blocked: { key: "k", name: "clippy", via: "goal" } });
    expect(goal).toEqual({ tone: "danger", message: 'The goal can\'t finish: local CI step "clippy" failed.', action: true });
    expect(ciBanner({ ...base, blocked: { key: "k", name: "fmt", via: "push" } })?.message).toContain("push is held back");
  });
  it("shows a run in flight and stays quiet otherwise", () => {
    expect(ciBanner({ ...base, running: { name: "tests", index: 2, total: 5 } })?.message).toBe("Local CI: tests (2/5)…");
    expect(ciBanner({ ...base, ok: true })).toBeNull();
    expect(ciBanner(null)).toBeNull();
    expect(ciBanner(undefined)).toBeNull();
  });
  it("prefers the block over a run that is re-checking", () => {
    const both = ciBanner({ ...base, running: { name: "x", index: 1, total: 2 }, blocked: { key: "k", name: "y", via: "goal" } });
    expect(both?.tone).toBe("danger");
  });
});
