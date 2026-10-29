import { describe, expect, it } from "vitest";

import {
  DEFAULT_DOCK_PREFS,
  isDismissed,
  moveDock,
  normalizeOrder,
  parseDockPrefs,
  sessionDockState,
  taskSignature,
  trimSessions,
} from "./composerDocks";

describe("parseDockPrefs", () => {
  it("falls back to the defaults for nothing, junk and wrong shapes", () => {
    for (const raw of [null, "", "not json", "null", "[]"]) {
      expect(parseDockPrefs(raw)).toEqual(DEFAULT_DOCK_PREFS);
    }
  });

  it("keeps what is valid and drops what is not", () => {
    const prefs = parseDockPrefs(
      JSON.stringify({
        placement: "panel",
        order: ["spend", "bogus", "spend"],
        sessions: { a: { collapsed: ["tasks", "x"], dismissed: { tasks: "sig", nope: "sig", spend: 3 } }, b: 7 },
      }),
    );
    expect(prefs.placement).toBe("panel");
    expect(prefs.order).toEqual(["spend", "tasks"]);
    expect(prefs.sessions).toEqual({ a: { collapsed: ["tasks"], dismissed: { tasks: "sig" } } });
  });
});

describe("dock order", () => {
  it("repairs a stored order so every dock appears once", () => {
    expect(normalizeOrder(["spend"])).toEqual(["spend", "tasks"]);
    expect(normalizeOrder(undefined)).toEqual(["tasks", "spend"]);
  });

  it("moves a dock one step and stops at the ends", () => {
    expect(moveDock(["tasks", "spend"], "tasks", 1)).toEqual(["spend", "tasks"]);
    expect(moveDock(["tasks", "spend"], "tasks", -1)).toEqual(["tasks", "spend"]);
    expect(moveDock(["tasks", "spend"], "spend", 1)).toEqual(["tasks", "spend"]);
  });
});

describe("dismissal", () => {
  const tasks = (...titles: string[]) => titles.map((title) => ({ title, status: "pending" as const }));

  it("hides a dock only while its content is the one that was hidden", () => {
    const state = { collapsed: [], dismissed: { tasks: taskSignature(tasks("a", "b")) } };
    expect(isDismissed(state, "tasks", taskSignature(tasks("a", "b")))).toBe(true);
    expect(isDismissed(state, "tasks", taskSignature(tasks("a", "b", "c")))).toBe(false);
    expect(isDismissed(state, "spend", "spend")).toBe(false);
  });

  it("does not let a task's status change bring a hidden dock back", () => {
    const before = taskSignature([{ title: "a", status: "pending" }]);
    const after = taskSignature([{ title: "a", status: "done" }]);
    expect(after).toBe(before);
  });

  it("reads an unknown session as untouched", () => {
    expect(sessionDockState(DEFAULT_DOCK_PREFS, "nope")).toEqual({ collapsed: [], dismissed: {} });
  });
});

describe("trimSessions", () => {
  it("keeps the touched session and the newest others", () => {
    const sessions = Object.fromEntries(Array.from({ length: 90 }, (_, i) => [`s${i}`, { collapsed: [], dismissed: {} }]));
    const trimmed = trimSessions(sessions, "s89");
    expect(Object.keys(trimmed)).toHaveLength(80);
    expect(trimmed.s89).toBeDefined();
    expect(trimmed.s0).toBeUndefined();
    expect(trimmed.s10).toBeDefined();
  });
});
