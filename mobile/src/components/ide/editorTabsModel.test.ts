import { describe, expect, it } from "vitest";

import { EMPTY_TABS, activeTab, editorTabsReducer, tabId, type EditorTabsAction, type EditorTabsState } from "./editorTabsModel";

const run = (actions: EditorTabsAction[], from: EditorTabsState = EMPTY_TABS) => actions.reduce(editorTabsReducer, from);
const open = (path: string, pinned = false, kind: "diff" | "file" = "diff"): EditorTabsAction => ({
  type: "open",
  kind,
  sessionId: "s",
  path,
  pinned,
});

describe("preview tabs", () => {
  it("single click opens a preview tab", () => {
    const state = run([open("a.ts")]);
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0].preview).toBe(true);
    expect(activeTab(state)?.path).toBe("a.ts");
  });

  it("the next single click replaces the preview in place", () => {
    const state = run([open("a.ts"), open("b.ts"), open("c.ts")]);
    expect(state.tabs.map((t) => t.path)).toEqual(["c.ts"]);
  });

  it("a double click pins, and a pinned tab survives later previews", () => {
    const state = run([open("a.ts"), open("a.ts", true), open("b.ts"), open("c.ts")]);
    expect(state.tabs.map((t) => [t.path, t.preview])).toEqual([
      ["a.ts", false],
      ["c.ts", true],
    ]);
  });

  it("previews open to the right of pinned tabs and keep the preview slot's position", () => {
    const state = run([open("a.ts", true), open("b.ts"), open("c.ts", true), open("d.ts")]);
    expect(state.tabs.map((t) => t.path)).toEqual(["a.ts", "d.ts", "c.ts"]);
  });

  it("an edit pins the preview", () => {
    const state = run([open("a.ts"), { type: "pin", id: tabId("diff", "s", "a.ts") }, open("b.ts")]);
    expect(state.tabs.map((t) => t.path)).toEqual(["a.ts", "b.ts"]);
    expect(state.tabs[0].preview).toBe(false);
  });

  it("reopening the same file activates it instead of duplicating, and a pinned open upgrades a preview", () => {
    const state = run([open("a.ts"), open("b.ts", true), open("a.ts", true)]);
    expect(state.tabs).toHaveLength(2);
    expect(state.tabs.find((t) => t.path === "a.ts")?.preview).toBe(false);
    expect(activeTab(state)?.path).toBe("a.ts");
  });

  it("a file view and a diff of the same path are different tabs", () => {
    const state = run([open("a.ts", true), open("a.ts", true, "file")]);
    expect(state.tabs).toHaveLength(2);
  });
});

describe("closing", () => {
  it("closing the active tab lands on the right neighbour, else the left", () => {
    const base = run([open("a.ts", true), open("b.ts", true), open("c.ts", true)]);
    const mid = editorTabsReducer({ ...base, activeId: tabId("diff", "s", "b.ts") }, { type: "close", id: tabId("diff", "s", "b.ts") });
    expect(activeTab(mid)?.path).toBe("c.ts");
    const last = editorTabsReducer(base, { type: "close", id: tabId("diff", "s", "c.ts") });
    expect(activeTab(last)?.path).toBe("b.ts");
    expect(editorTabsReducer(editorTabsReducer(last, { type: "close", id: tabId("diff", "s", "b.ts") }), { type: "close", id: tabId("diff", "s", "a.ts") }).activeId).toBeNull();
  });

  it("closing another tab keeps the active one", () => {
    const base = run([open("a.ts", true), open("b.ts", true)]);
    const state = editorTabsReducer(base, { type: "close", id: tabId("diff", "s", "a.ts") });
    expect(activeTab(state)?.path).toBe("b.ts");
  });

  it("drop removes tabs whose file left the change set and ignores unknown ids", () => {
    const base = run([open("a.ts", true), open("b.ts", true)]);
    const state = editorTabsReducer(base, { type: "drop", ids: [tabId("diff", "s", "b.ts"), "nope"] });
    expect(state.tabs.map((t) => t.path)).toEqual(["a.ts"]);
    expect(editorTabsReducer(state, { type: "drop", ids: ["nope"] })).toBe(state);
  });
});
