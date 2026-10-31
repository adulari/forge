import { describe, expect, it } from "vitest";

import { INITIAL_WORKBENCH_STATE, createWorkbenchSurface, workbenchReducer, type WorkbenchState } from "./model";

const file = (path: string, preview = false) =>
  createWorkbenchSurface({ kind: "files", sessionId: "s1", resourceId: path, title: path, preview });

function open(state: WorkbenchState, surface: ReturnType<typeof file>): WorkbenchState {
  return workbenchReducer(state, { type: "open", surface });
}

describe("preview tabs", () => {
  it("replaces the previous preview tab in place instead of adding one", () => {
    let state = open(INITIAL_WORKBENCH_STATE, file("a.ts", true));
    state = open(state, file("b.ts", true));
    expect(state.right.tabs.map((tab) => tab.resourceId)).toEqual(["b.ts"]);
    expect(state.right.activeId).toBe(file("b.ts").id);
  });

  it("keeps a pinned tab and opens the next preview beside it", () => {
    let state = open(INITIAL_WORKBENCH_STATE, file("a.ts", true));
    state = workbenchReducer(state, { type: "pin", placement: "right", id: file("a.ts").id });
    state = open(state, file("b.ts", true));
    expect(state.right.tabs.map((tab) => [tab.resourceId, Boolean(tab.preview)])).toEqual([
      ["a.ts", false],
      ["b.ts", true],
    ]);
  });

  it("does not demote a pinned tab when it is opened as a preview again", () => {
    let state = open(INITIAL_WORKBENCH_STATE, file("a.ts"));
    state = open(state, file("a.ts", true));
    expect(state.right.tabs[0]?.preview).toBeUndefined();
  });

  it("opening a file normally never replaces a preview tab", () => {
    let state = open(INITIAL_WORKBENCH_STATE, file("a.ts", true));
    state = open(state, file("b.ts"));
    expect(state.right.tabs).toHaveLength(2);
  });

  it("ignores a pin for a tab that is not a preview", () => {
    const state = open(INITIAL_WORKBENCH_STATE, file("a.ts"));
    expect(workbenchReducer(state, { type: "pin", placement: "right", id: file("a.ts").id })).toBe(state);
  });
});
