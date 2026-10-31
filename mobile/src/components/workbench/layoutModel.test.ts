import { describe, expect, it } from "vitest";

import {
  DEFAULT_WEIGHTS,
  MAX_PANES,
  MIN_PANE_WIDTH,
  addPane,
  clampBottomHeight,
  clampRightWidth,
  cycleFocus,
  initialLayout,
  layoutKind,
  migrateLegacySplit,
  minWeight,
  openInFocused,
  openInNewPane,
  openInPane,
  paneCount,
  paneGeometry,
  parseLayout,
  promoteFirstSecondary,
  removePane,
  resizeBoundary,
  serializeLayout,
  setSlot,
  swapSecondary,
  swapWithPrimary,
  toggleMaximize,
  visiblePaneCount,
  withSecondary,
  indexOfSlotId,
  nextPaneSize,
  secondaryRenderOrder,
  type PaneLayoutState,
} from "./layoutModel";

function layoutWith(secondary: (string | null)[], focus = 0): PaneLayoutState {
  return { ...withSecondary(initialLayout(), secondary), focus };
}

describe("pane add / remove / swap", () => {
  it("adds panes up to the cap and focuses the new one", () => {
    let state = initialLayout();
    state = addPane(state, "b");
    state = addPane(state, "c");
    state = addPane(state, null);
    expect(state.secondary).toEqual(["b", "c", null]);
    expect(state.focus).toBe(3);
    expect(paneCount(state)).toBe(MAX_PANES);
    expect(addPane(state, "e")).toBe(state);
  });

  it("maps the pane count to a layout kind", () => {
    expect([1, 2, 3, 4].map(layoutKind)).toEqual(["single", "cols2", "cols3", "grid"]);
  });

  it("removes a secondary pane and keeps focus on a surviving pane", () => {
    const state = layoutWith(["b", "c", "d"], 2);
    const after = removePane(state, 2);
    expect(after.secondary).toEqual(["b", "d"]);
    expect(after.focus).toBe(1);
    expect(removePane(layoutWith(["b", "c"], 2), 1).focus).toBe(1);
    expect(removePane(state, 0)).toBe(state);
  });

  it("clears a maximized pane when it is removed", () => {
    const state = { ...layoutWith(["b", "c"], 1), maximized: 1 };
    expect(removePane(state, 1).maximized).toBeNull();
    expect(removePane({ ...state, maximized: 2 }, 1).maximized).toBe(1);
  });

  it("promotes the first secondary session when pane 0 closes", () => {
    const result = promoteFirstSecondary(layoutWith(["b", "c"], 1));
    expect(result.promote).toBe("b");
    expect(result.state.secondary).toEqual(["c"]);
    expect(result.state.focus).toBe(0);
    expect(promoteFirstSecondary(initialLayout()).promote).toBeNull();
  });

  it("swaps a secondary pane with the routed one", () => {
    const result = swapWithPrimary(layoutWith(["b", "c"]), "a", 2);
    expect(result.promote).toBe("c");
    expect(result.state.secondary).toEqual(["b", "a"]);
    expect(swapWithPrimary(layoutWith(["b"]), "a", 5).promote).toBeNull();
  });

  it("swaps two secondary panes and follows focus", () => {
    const after = swapSecondary(layoutWith(["b", "c", "d"], 1), 1, 3);
    expect(after.secondary).toEqual(["d", "c", "b"]);
    expect(after.focus).toBe(3);
    expect(swapSecondary(layoutWith(["b"]), 1, 1).secondary).toEqual(["b"]);
  });

  it("cycles focus both ways and ignores a single pane", () => {
    const state = layoutWith(["b", "c"], 2);
    expect(cycleFocus(state, 1).focus).toBe(0);
    expect(cycleFocus(layoutWith(["b", "c"], 0), -1).focus).toBe(2);
    expect(cycleFocus(initialLayout(), 1)).toEqual(initialLayout());
  });

  it("toggles maximize on a pane and back", () => {
    const state = layoutWith(["b"], 0);
    const max = toggleMaximize(state, 1);
    expect(max.maximized).toBe(1);
    expect(max.focus).toBe(1);
    expect(toggleMaximize(max, 1).maximized).toBeNull();
    expect(toggleMaximize(initialLayout(), 0).maximized).toBeNull();
  });
});

describe("opening sessions into panes", () => {
  it("never shows a session twice", () => {
    const state = layoutWith(["b", "c"], 0);
    expect(openInFocused(state, "a", "c")).toEqual({ state: { ...state, focus: 2 }, routeTo: null });
    expect(openInFocused(state, "a", "a").state.focus).toBe(0);
    expect(openInNewPane(state, "a", "b").focus).toBe(1);
  });

  it("opens into the focused secondary pane, or routes when pane 0 has focus", () => {
    const secondaryFocus = openInFocused(layoutWith(["b", "c"], 2), "a", "z");
    expect(secondaryFocus.state.secondary).toEqual(["b", "z"]);
    expect(secondaryFocus.routeTo).toBeNull();
    expect(openInFocused(layoutWith(["b"], 0), "a", "z").routeTo).toBe("z");
  });

  it("opens in a new pane, and reuses the last one when the grid is full", () => {
    expect(openInNewPane(initialLayout(), "a", "b").secondary).toEqual(["b"]);
    const full = layoutWith(["b", "c", "d"], 0);
    const after = openInNewPane(full, "a", "z");
    expect(after.secondary).toEqual(["b", "c", "z"]);
    expect(after.focus).toBe(3);
  });

  it("drops a session on a pane, moving it out of any other pane", () => {
    const state = layoutWith(["b", "c"], 0);
    const moved = openInPane(state, "a", 1, "c");
    expect(moved.state.secondary).toEqual(["c", null]);
    expect(moved.state.focus).toBe(1);
    expect(openInPane(state, "a", 0, "z")).toMatchObject({ routeTo: "z" });
  });
});

describe("sizing", () => {
  it("hides panes that cannot fit at the minimum width", () => {
    expect(visiblePaneCount(3, MIN_PANE_WIDTH * 3)).toBe(3);
    expect(visiblePaneCount(3, MIN_PANE_WIDTH * 2 + 10)).toBe(2);
    expect(visiblePaneCount(2, MIN_PANE_WIDTH - 1)).toBe(1);
    expect(visiblePaneCount(4, MIN_PANE_WIDTH * 2)).toBe(4);
    expect(visiblePaneCount(4, MIN_PANE_WIDTH * 2 - 1)).toBe(1);
    expect(visiblePaneCount(1, 5000)).toBe(1);
  });

  it("moves a seam while keeping both neighbours above the minimum", () => {
    expect(resizeBoundary([0.5, 0.5], 0, 0.1, 0.25)).toEqual([0.6, 0.4]);
    expect(resizeBoundary([0.5, 0.5], 0, 0.9, 0.25)).toEqual([0.75, 0.25]);
    expect(resizeBoundary([0.5, 0.5], 0, -0.9, 0.25)).toEqual([0.25, 0.75]);
    const three = resizeBoundary([1 / 3, 1 / 3, 1 / 3], 1, 0.05, 0.2);
    expect(three.reduce((a, b) => a + b, 0)).toBeCloseTo(1);
    expect(three[0]).toBeCloseTo(1 / 3);
  });

  it("derives the minimum share from the pixel minimum", () => {
    expect(minWeight(1600, 2)).toBeCloseTo(MIN_PANE_WIDTH / 1600);
    expect(minWeight(600, 3)).toBeCloseTo(1 / 3);
  });

  it("clamps dock sizes to the window", () => {
    expect(clampRightWidth(100, 1600, 232)).toBe(240);
    expect(clampRightWidth(2000, 1600, 232)).toBe(760);
    expect(clampRightWidth(2000, 1100, 232)).toBe(388);
    expect(clampBottomHeight(10, 900)).toBe(110);
    expect(clampBottomHeight(900, 900)).toBe(540);
    expect(clampBottomHeight(900, 2000)).toBe(560);
  });
});

describe("pane geometry", () => {
  it("lays out two and three columns that tile the container", () => {
    const two = paneGeometry(2, 1000, 600, { ...DEFAULT_WEIGHTS, cols2: [0.4, 0.6] }, null);
    expect(two.kind).toBe("cols2");
    expect(two.rects[0]).toEqual({ left: 0, top: 0, width: 400, height: 600 });
    expect(two.rects[1]).toEqual({ left: 400, top: 0, width: 600, height: 600 });
    expect(two.seams).toEqual([{ key: "cols2", orientation: "vertical", boundary: 0, at: 400 }]);

    const three = paneGeometry(3, 1500, 600, DEFAULT_WEIGHTS, null);
    expect(three.rects.map((rect) => rect?.width)).toEqual([500, 500, 500]);
    expect(three.seams).toHaveLength(2);
  });

  it("builds a 2x2 grid with a vertical and a horizontal seam", () => {
    const grid = paneGeometry(4, 1000, 800, { ...DEFAULT_WEIGHTS, grid: [0.5, 0.25] }, null);
    expect(grid.kind).toBe("grid");
    expect(grid.rects[3]).toEqual({ left: 500, top: 200, width: 500, height: 600 });
    expect(grid.seams.map((seam) => seam.orientation).sort()).toEqual(["horizontal", "vertical"]);
  });

  it("falls back to fewer panes on a narrow window and keeps hidden ones mounted", () => {
    const narrow = paneGeometry(3, MIN_PANE_WIDTH * 2, 600, DEFAULT_WEIGHTS, null);
    expect(narrow.visible).toBe(2);
    expect(narrow.rects[2]).toBeNull();
    const tiny = paneGeometry(4, MIN_PANE_WIDTH + 100, 600, DEFAULT_WEIGHTS, null);
    expect(tiny.visible).toBe(1);
    expect(tiny.rects.filter(Boolean)).toHaveLength(1);
  });

  it("shows only the maximized pane", () => {
    const geometry = paneGeometry(3, 1500, 600, DEFAULT_WEIGHTS, 2);
    expect(geometry.rects[2]).toEqual({ left: 0, top: 0, width: 1500, height: 600 });
    expect(geometry.rects[0]).toBeNull();
    expect(geometry.seams).toEqual([]);
  });

  it("lifts a saved weight that is below the minimum on this window", () => {
    const geometry = paneGeometry(2, 1000, 600, { ...DEFAULT_WEIGHTS, cols2: [0.05, 0.95] }, null);
    expect(geometry.rects[0]?.width ?? 0).toBeGreaterThanOrEqual(MIN_PANE_WIDTH - 1);
  });
});

describe("persistence", () => {
  it("round-trips a layout through its serialised form", () => {
    const state: PaneLayoutState = {
      ...layoutWith(["b", null]),
      weights: { cols2: [0.3, 0.7], cols3: [0.2, 0.3, 0.5], grid: [0.4, 0.6] },
      dock: { right: 500, bottom: 220 },
    };
    const parsed = parseLayout(JSON.parse(JSON.stringify(serializeLayout(state))));
    expect(parsed.secondary).toEqual(["b", null]);
    expect(parsed.weights.grid).toEqual([0.4, 0.6]);
    expect(parsed.dock).toEqual({ right: 500, bottom: 220 });
    expect(parsed.focus).toBe(0);
  });

  it("falls back to defaults for malformed input instead of throwing", () => {
    expect(parseLayout(null)).toEqual(initialLayout());
    expect(parseLayout("nope")).toEqual(initialLayout());
    const parsed = parseLayout({
      secondary: ["b", 5, "c", "d", "e"],
      weights: { cols2: [0.9], cols3: "x", grid: [2, -1] },
      dock: { bottom: "tall", right: "wide" },
    });
    expect(parsed.secondary).toEqual(["b", "c", "d"]);
    expect(parsed.weights).toEqual(initialLayout().weights);
    expect(parsed.dock).toEqual({ right: null, bottom: null });
  });

  it("migrates the old forge.splitPanes entry", () => {
    expect(migrateLegacySplit(JSON.stringify({ on: true, secondary: "sess-2" }))?.secondary).toEqual(["sess-2"]);
    expect(migrateLegacySplit(JSON.stringify({ on: true, secondary: null }))?.secondary).toEqual([null]);
    expect(migrateLegacySplit(JSON.stringify({ on: false, secondary: "sess-2" }))?.secondary).toEqual([]);
    expect(migrateLegacySplit("{broken")).toBeNull();
    expect(migrateLegacySplit(JSON.stringify({ secondary: "x" }))).toBeNull();
    expect(migrateLegacySplit(null)).toBeNull();
  });
});

describe("stable slot ids", () => {
  const ids = (state: PaneLayoutState) => state.slotIds;

  it("keeps ids parallel to the secondary list through add, remove, swap and set", () => {
    let state = addPane(addPane(addPane(initialLayout(), "a"), "b"), "c");
    expect(ids(state)).toEqual([1, 2, 3]);
    state = removePane(state, 1);
    expect(state.secondary).toEqual(["b", "c"]);
    expect(ids(state)).toEqual([2, 3]);
    state = setSlot(state, 1, "x");
    expect(ids(state)).toEqual([2, 3]);
    state = swapSecondary(state, 1, 2);
    expect(state.secondary).toEqual(["c", "x"]);
    expect(ids(state)).toEqual([3, 2]);
    expect(state.slotIds).toHaveLength(state.secondary.length);
  });

  it("never reuses an id after a pane closes", () => {
    let state = addPane(addPane(initialLayout(), "a"), "b");
    state = removePane(state, 2);
    state = addPane(state, "c");
    expect(ids(state)).toEqual([1, 3]);
  });

  it("drops the first id when pane 0 is closed", () => {
    const state = addPane(addPane(initialLayout(), "a"), "b");
    const { state: next, promote } = promoteFirstSecondary(state);
    expect(promote).toBe("a");
    expect(ids(next)).toEqual([2]);
  });

  it("assigns 1..n on parse and leaves the stored shape unchanged", () => {
    const stored = { secondary: ["b", null, "d"], weights: { cols2: [0.5, 0.5], cols3: [0.2, 0.3, 0.5], grid: [0.4, 0.6] }, dock: { right: 500, bottom: null } };
    const parsed = parseLayout(JSON.parse(JSON.stringify(stored)));
    expect(parsed.slotIds).toEqual([1, 2, 3]);
    expect(JSON.parse(JSON.stringify(serializeLayout(parsed)))).toEqual(stored);
    expect(Object.keys(serializeLayout(parsed)).sort()).toEqual(["dock", "secondary", "weights"]);
  });

  it("a preset keeps the id of a pane that stays", () => {
    const state = addPane(addPane(initialLayout(), "a"), "b");
    const next = withSecondary(state, ["b", "z"]);
    expect(next.slotIds[0]).toBe(2);
    expect(next.slotIds[1]).toBe(state.nextSlotId);
  });

  it("resolves a slot id to its current index", () => {
    let state = addPane(addPane(addPane(initialLayout(), "a"), "b"), "c");
    state = removePane(state, 1);
    expect(indexOfSlotId(state, 0)).toBe(0);
    expect(indexOfSlotId(state, 2)).toBe(1);
    expect(indexOfSlotId(state, 3)).toBe(2);
    expect(indexOfSlotId(state, 1)).toBe(-1);
  });

  it("remount proof: closing the first of three panes leaves the survivors' React keys, and their relative order, untouched", () => {
    const before = addPane(addPane(addPane(initialLayout(), "a"), "b"), "c");
    const keysBefore = secondaryRenderOrder(before).map((p) => p.slotId);
    const after = removePane(before, 1);
    const rendered = secondaryRenderOrder(after);
    const keysAfter = rendered.map((p) => p.slotId);
    expect(keysAfter).toEqual(keysBefore.slice(1));
    expect(rendered.map((p) => p.slot)).toEqual(["b", "c"]);
    // The pane that was index 2 is now index 1, but it is still the same keyed element.
    expect(rendered[0]).toMatchObject({ slot: "b", slotId: 2, index: 1 });
  });

  it("renders in slot-id order even after a swap, so a swap never moves a live node", () => {
    const before = addPane(addPane(initialLayout(), "a"), "b");
    const after = swapSecondary(before, 1, 2);
    expect(secondaryRenderOrder(after).map((p) => p.slot)).toEqual(["a", "b"]);
    expect(secondaryRenderOrder(after).map((p) => p.index)).toEqual([2, 1]);
  });
});

describe("zero-size guard", () => {
  it("keeps the last real size when the container collapses to zero", () => {
    const real = { width: 1400, height: 800 };
    expect(nextPaneSize(real, 0, 800)).toBe(real);
    expect(nextPaneSize(real, 1400, 0)).toBe(real);
    expect(nextPaneSize(null, 0, 0)).toBeNull();
  });

  it("a collapsed layout would otherwise hide every secondary pane and push focus to pane 0", () => {
    expect(visiblePaneCount(3, 0)).toBe(1);
    expect(visiblePaneCount(3, 1400)).toBe(3);
  });

  it("takes a real new size and reuses the object for an identical one", () => {
    const real = { width: 1400, height: 800 };
    expect(nextPaneSize(real, 1400, 800)).toBe(real);
    expect(nextPaneSize(real, 1000, 800)).toEqual({ width: 1000, height: 800 });
    expect(nextPaneSize(null, 1000, 800)).toEqual({ width: 1000, height: 800 });
  });
});
