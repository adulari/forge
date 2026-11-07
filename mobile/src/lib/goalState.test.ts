import { describe, expect, it } from "vitest";

import {
  budgetFraction,
  compactTokens,
  contractHeadline,
  contractRows,
  deriveNoteGoal,
  envelopeFraction,
  formatElapsed,
  goalActions,
  goalInput,
  goalMeta,
  goalStartCommand,
  liveElapsedSecs,
  resumeLine,
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

describe("envelope", () => {
  const envelope = { unit: "usd" as const, used: 1.5, slice: 5, label: "spend", meter: "envelope $1.50/$5.00", spent: false, unverified: false, resets_at: null };

  it("carries the host's envelope and reports how full it is", () => {
    const goal = resolveGoal({ goal: wire({ envelope }), notes: [] });
    expect(goal?.envelope?.meter).toBe("envelope $1.50/$5.00");
    expect(goal && envelopeFraction(goal)).toBeCloseTo(0.3);
  });

  it("is absent from an older host's goal", () => {
    const goal = resolveGoal({ goal: wire(), notes: [] });
    expect(goal?.envelope).toBeNull();
    expect(goal && envelopeFraction(goal)).toBeNull();
  });
});

describe("resumeLine", () => {
  const spent = { unit: "percent" as const, used: 21, slice: 20, label: "claude weekly", meter: "envelope 21/20% claude weekly", spent: true, unverified: false, resets_at: 0 };
  const now = new Date(2026, 10, 6, 23, 30).getTime();
  const at = new Date(2026, 10, 7, 4, 0).getTime() / 1000;

  it("says when a goal waiting for its window continues by itself", () => {
    const goal = resolveGoal({ goal: wire({ state: "paused", envelope: { ...spent, resumes_at: at } }), notes: [] });
    expect(goal && resumeLine(goal, now)).toBe("Resumes at 04:00");
  });

  it("stays silent when it will not resume by itself", () => {
    const held = resolveGoal({ goal: wire({ state: "paused", envelope: spent }), notes: [] });
    expect(held && resumeLine(held, now)).toBeNull();
    const running = resolveGoal({ goal: wire({ state: "active", envelope: { ...spent, resumes_at: at } }), notes: [] });
    expect(running && resumeLine(running, now)).toBeNull();
  });

  it("adds the weekday when the reset is a day or more away", () => {
    const far = new Date(2026, 10, 9, 9, 0).getTime() / 1000;
    const goal = resolveGoal({ goal: wire({ state: "paused", envelope: { ...spent, resumes_at: far } }), notes: [] });
    expect(goal && resumeLine(goal, now)).toBe("Resumes at Mon 09:00");
  });
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

describe("contract", () => {
  const contract = {
    headline: "1/3 pass",
    approved: true,
    rounds: 2,
    passed: 1,
    total: 3,
    checked_at: 1_700_000_000,
    assertions: [
      { id: 1, text: "a exists", command: "test -f a.txt", expect: "exit 0", state: "pass" as const, reason: "", proof: "" },
      { id: 2, text: "b says ready", command: "cat b.txt", expect: "", state: "fail" as const, reason: 'output does not contain "ready"', proof: "pending\nnot yet\n" },
      { id: 3, text: "c exists", command: "test -f c.txt", expect: "", state: "pending" as const, reason: "", proof: "" },
    ],
  };

  it("turns the host's contract into table rows with a mark per state", () => {
    const goal = resolveGoal({ goal: wire({ contract }), notes: [] });
    const rows = goal && contractRows(goal);
    expect(rows?.map((r) => r.mark)).toEqual(["✓", "✗", "·"]);
    expect(rows?.[1].detail).toBe('output does not contain "ready" · not yet');
    expect(rows?.[0].detail).toBe("");
    expect(goal && contractHeadline(goal)).toBe("Contract · 1/3 pass");
  });

  it("is absent for a goal without a contract or from an older host", () => {
    const goal = resolveGoal({ goal: wire(), notes: [] });
    expect(goal && contractRows(goal)).toBeNull();
    expect(goal && contractHeadline(goal)).toBeNull();
  });
});

describe("goalStartCommand", () => {
  it("asks for a contract only when the switch is on", () => {
    expect(goalStartCommand("  ship it ", false)).toBe("/goal ship it");
    expect(goalStartCommand("ship it", true)).toBe("/goal --contract ship it");
  });
});
