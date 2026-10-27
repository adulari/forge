import { describe, expect, it } from "vitest";

import {
  INITIAL_WORKBENCH_STATE,
  activeWorkbenchSurface,
  createWorkbenchSurface,
  parseWorkbenchState,
  restorableWorkbenchState,
  workbenchReducer,
  type WorkbenchState,
} from "./model";

function reduce(
  state: WorkbenchState,
  action: Parameters<typeof workbenchReducer>[1],
): WorkbenchState {
  return workbenchReducer(state, action);
}

describe("workbench surface model", () => {
  it("keeps opened right surfaces as selectable tabs", () => {
    const usage = createWorkbenchSurface({ kind: "usage" });
    const git = createWorkbenchSurface({ kind: "git" });

    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: usage });
    state = reduce(state, { type: "open", surface: git });

    expect(state.right.tabs.map((surface) => surface.kind)).toEqual(["usage", "git"]);
    expect(activeWorkbenchSurface(state, "right")).toEqual(git);

    state = reduce(state, { type: "activate", placement: "right", id: usage.id });
    expect(activeWorkbenchSurface(state, "right")).toEqual(usage);
  });

  it("hides an active lane without throwing away its tabs", () => {
    const usage = createWorkbenchSurface({ kind: "usage" });
    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "toggle", surface: usage });
    state = reduce(state, { type: "toggle", surface: usage });

    expect(state.right.visible).toBe(false);
    expect(state.right.tabs).toEqual([usage]);
    expect(activeWorkbenchSurface(state, "right")).toBeNull();

    state = reduce(state, { type: "toggle", surface: usage });
    expect(activeWorkbenchSurface(state, "right")).toEqual(usage);
  });

  it("keeps bottom and right surfaces independent", () => {
    const git = createWorkbenchSurface({ kind: "git" });
    const terminal = createWorkbenchSurface({ kind: "terminal" });

    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: git });
    state = reduce(state, { type: "open", surface: terminal });
    state = reduce(state, { type: "hide", placement: "right" });

    expect(activeWorkbenchSurface(state, "right")).toBeNull();
    expect(activeWorkbenchSurface(state, "bottom")).toEqual(terminal);
  });

  it("selects the adjacent tab after closing the active surface", () => {
    const usage = createWorkbenchSurface({ kind: "usage" });
    const git = createWorkbenchSurface({ kind: "git" });
    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: usage });
    state = reduce(state, { type: "open", surface: git });
    state = reduce(state, { type: "close", placement: "right", id: git.id });

    expect(activeWorkbenchSurface(state, "right")).toEqual(usage);

    state = reduce(state, { type: "close", placement: "right", id: usage.id });
    expect(state.right).toEqual({ tabs: [], activeId: null, visible: false });
  });

  it("deduplicates a surface identity while allowing resource-specific tabs", () => {
    const first = createWorkbenchSurface({ kind: "terminal", resourceId: "one" });
    const renamed = createWorkbenchSurface({
      kind: "terminal",
      resourceId: "one",
      title: "API server",
    });
    const second = createWorkbenchSurface({ kind: "terminal", resourceId: "two" });

    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: first });
    state = reduce(state, { type: "open", surface: renamed });
    state = reduce(state, { type: "open", surface: second });

    expect(state.bottom.tabs).toHaveLength(2);
    expect(state.bottom.tabs[0]?.title).toBe("API server");
    expect(activeWorkbenchSurface(state, "bottom")).toEqual(second);
  });

  it("pins browser preview tabs to a session and resource in the right lane", () => {
    const first = createWorkbenchSurface({
      kind: "preview",
      sessionId: "session-a",
      resourceId: "tab-one",
    });
    const second = createWorkbenchSurface({
      kind: "preview",
      sessionId: "session-a",
      resourceId: "tab-two",
    });

    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: first });
    state = reduce(state, { type: "open", surface: second });

    expect(first.placement).toBe("right");
    expect(first.id).not.toBe(second.id);
    expect(state.right.tabs).toEqual([first, second]);
    expect(activeWorkbenchSurface(state, "right")).toEqual(second);
  });
});

describe("session docks and lane persistence", () => {
  it("registers Tasks, Agents and Plan as session-scoped right docks", () => {
    for (const kind of ["tasks", "agents", "plan"] as const) {
      const surface = createWorkbenchSurface({ kind });
      expect(surface.placement).toBe("right");
      expect(surface.sessionId).toBeNull();
    }
  });

  it("reorders tabs within a lane and ignores out-of-range moves", () => {
    const surfaces = (["usage", "git", "tasks"] as const).map((kind) => createWorkbenchSurface({ kind }));
    let state = INITIAL_WORKBENCH_STATE;
    for (const surface of surfaces) state = reduce(state, { type: "open", surface });
    state = reduce(state, { type: "reorder", placement: "right", from: 2, to: 0 });
    expect(state.right.tabs.map((tab) => tab.kind)).toEqual(["tasks", "usage", "git"]);
    expect(reduce(state, { type: "reorder", placement: "right", from: 0, to: 9 })).toBe(state);
  });

  it("restores only surfaces that follow focus, never one pinned to a session that may be gone", () => {
    const followed = createWorkbenchSurface({ kind: "git" });
    const pinned = createWorkbenchSurface({ kind: "preview", sessionId: "gone" });
    let state = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: followed });
    state = reduce(state, { type: "open", surface: pinned });
    const restored = restorableWorkbenchState(state);
    expect(restored.right.tabs).toEqual([followed]);
    expect(restored.right.activeId).toBe(followed.id);

    const onlyPinned = reduce(INITIAL_WORKBENCH_STATE, { type: "open", surface: pinned });
    expect(restorableWorkbenchState(onlyPinned).right.visible).toBe(false);
  });

  it("parses persisted lanes tolerantly", () => {
    const parsed = parseWorkbenchState({
      right: { tabs: [{ kind: "tasks" }, { kind: "nonsense" }, { kind: "terminal" }, { kind: "tasks" }], activeId: "tasks:_:_", visible: true },
      bottom: { tabs: [{ kind: "terminal" }], activeId: "missing", visible: true },
    });
    expect(parsed.right.tabs.map((tab) => tab.kind)).toEqual(["tasks"]);
    expect(parsed.right.visible).toBe(true);
    expect(parsed.bottom.activeId).toBe("terminal:_:_");
    expect(parseWorkbenchState(null)).toEqual(INITIAL_WORKBENCH_STATE);
  });
});
