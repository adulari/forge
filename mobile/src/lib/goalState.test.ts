import { describe, expect, it } from "vitest";

import {
  budgetFraction,
  compactTokens,
  deriveNoteGoal,
  formatElapsed,
  goalActions,
  goalInput,
  goalMeta,
  liveElapsedSecs,
  resolveGoal,
} from "./goalState";
import type { SnapshotGoal } from "./ws";

const wire = (over: Partial<SnapshotGoal> = {}): SnapshotGoal => ({
  objective: "ship the parser",
  state: "active",
  iteration: 3,
  tokens_used: 12_500,
  max_tokens: 50_000,
  max_minutes: 30,
  elapsed_secs: 90,
  synced_at: 1_000_000,
  gates: ["cargo test"],
  reason: null,
  pending: null,
  ...over,
});

describe("resolveGoal", () => {
  it("prefers the host's structured goal", () => {
    const goal = resolveGoal({ goal: wire(), notes: ["🎯 goal set — other"] });
    expect(goal?.structured).toBe(true);
    expect(goal?.objective).toBe("ship the parser");
  });

  it("falls back to the notes an older host sends, without verbs it cannot take", () => {
    const goal = resolveGoal({ notes: ["🎯 goal set — fix the build", "🎯 goal started"] });
    expect(goal).toMatchObject({ structured: false, phase: "active", objective: "fix the build" });
    expect(goalActions(goal!)).toEqual(["stop"]);
  });

  it("is null with no goal and after a noted stop", () => {
    expect(resolveGoal(null)).toBeNull();
    expect(resolveGoal({ notes: [] })).toBeNull();
    expect(deriveNoteGoal(["🎯 goal set — x", "🎯 goal stopped — budget"])).toBeNull();
  });
});

describe("goalActions", () => {
  const actions = (over: Partial<SnapshotGoal>) => goalActions(resolveGoal({ goal: wire(over), notes: [] })!);

  it("offers pause, edit and stop while running", () => {
    expect(actions({})).toEqual(["pause", "edit", "stop"]);
  });

  it("leaves only stop once a pause or clear is already pending", () => {
    expect(actions({ pending: "pause" })).toEqual(["stop"]);
  });

  it("offers resume, edit and clear when paused", () => {
    expect(actions({ state: "paused" })).toEqual(["resume", "edit", "clear"]);
  });

  it("offers only clear once the goal has ended", () => {
    for (const state of ["complete", "failed", "budget_limit", "cancelled"] as const) {
      expect(actions({ state })).toEqual(["clear"]);
    }
  });
});

describe("goalInput", () => {
  it("maps each action onto the input the host understands", () => {
    expect(goalInput("pause")).toEqual({ kind: "prompt", text: "/goal pause" });
    expect(goalInput("resume")).toEqual({ kind: "prompt", text: "/goal resume" });
    expect(goalInput("clear")).toEqual({ kind: "prompt", text: "/goal clear" });
    expect(goalInput("stop")).toEqual({ kind: "interrupt" });
    expect(goalInput("edit", "  a sharper goal ")).toEqual({ kind: "prompt", text: "/goal edit a sharper goal" });
  });
});

describe("goal readouts", () => {
  it("formats tokens and elapsed time compactly", () => {
    expect(compactTokens(999)).toBe("999");
    expect(compactTokens(12_500)).toBe("12.5k");
    expect(compactTokens(50_000)).toBe("50k");
    expect(compactTokens(2_000_000)).toBe("2M");
    expect(formatElapsed(9)).toBe("9s");
    expect(formatElapsed(250)).toBe("4m 10s");
    expect(formatElapsed(7_500)).toBe("2h 5m");
  });

  it("keeps an active goal's clock running past the host's last reading", () => {
    const goal = resolveGoal({ goal: wire(), notes: [] })!;
    expect(liveElapsedSecs(goal, 1_000_000 + 30_000)).toBe(120);
    const paused = resolveGoal({ goal: wire({ state: "paused" }), notes: [] })!;
    expect(liveElapsedSecs(paused, 9_999_999)).toBe(90);
  });

  it("reports the budget share and a one-line meta", () => {
    const goal = resolveGoal({ goal: wire(), notes: [] })!;
    expect(budgetFraction(goal)).toBeCloseTo(0.25);
    expect(budgetFraction({ ...goal, maxTokens: null })).toBeNull();
    expect(goalMeta(goal, 1_000_000)).toBe("turn 3 · 12.5k / 50k tok · 1m 30s / 30m · 1 gate");
  });
});
