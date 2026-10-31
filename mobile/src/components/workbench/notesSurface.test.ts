import { describe, expect, it } from "vitest";

import {
  createWorkbenchSurface,
  INITIAL_WORKBENCH_STATE,
  parseWorkbenchState,
  restorableWorkbenchState,
  workbenchReducer,
  WORKBENCH_SURFACE_DEFINITIONS,
} from "./model";

describe("notes surface", () => {
  it("is a right-hand dock that follows the project, not one session", () => {
    expect(WORKBENCH_SURFACE_DEFINITIONS.notes).toMatchObject({ placement: "right", sessionScoped: false });
    expect(createWorkbenchSurface({ kind: "notes" }).id).toBe("notes:_:_");
  });

  it("toggles open and closed like the other docks", () => {
    const surface = createWorkbenchSurface({ kind: "notes" });
    const open = workbenchReducer(INITIAL_WORKBENCH_STATE, { type: "toggle", surface });
    expect(open.right).toMatchObject({ activeId: surface.id, visible: true });
    expect(workbenchReducer(open, { type: "toggle", surface }).right.visible).toBe(false);
  });

  it("survives a relaunch", () => {
    const surface = createWorkbenchSurface({ kind: "notes" });
    const open = workbenchReducer(INITIAL_WORKBENCH_STATE, { type: "open", surface });
    const restored = parseWorkbenchState(JSON.parse(JSON.stringify(restorableWorkbenchState(open))));
    expect(restored.right.tabs.map((tab) => tab.kind)).toEqual(["notes"]);
    expect(restored.right.visible).toBe(true);
  });
});
