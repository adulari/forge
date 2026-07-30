import { describe, expect, it } from "vitest";

import {
  INITIAL_WORKBENCH_STATE,
  activeWorkbenchSurface,
  createWorkbenchSurface,
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
