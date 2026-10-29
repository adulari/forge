import { describe, expect, it } from "vitest";

import {
  ALL_VIEW_CAP,
  CATEGORY_VIEW_CAP,
  PAGE_STEP,
  buildSections,
  cycleCategory,
  filesEmptyState,
  filesHeader,
  isImeComposing,
  pageIndex,
  selectableRows,
  stepIndex,
  type PaletteSection,
} from "./sections";

const items = (section: PaletteSection, count: number, disabledFrom = Infinity) =>
  Array.from({ length: count }, (_, i) => ({ id: `${section}:${i}`, section, disabled: i >= disabledFrom }));

describe("cycleCategory", () => {
  it("walks the chips forwards and backwards with wraparound", () => {
    expect(cycleCategory("all", 1)).toBe("sessions");
    expect(cycleCategory("settings", 1)).toBe("all");
    expect(cycleCategory("all", -1)).toBe("settings");
    expect(cycleCategory("files", -1)).toBe("commands");
  });
});

describe("buildSections", () => {
  const ranked = [...items("sessions", 9), ...items("commands", 3), ...items("files", 2)];

  it("caps each section in the All view and reports how many are hidden", () => {
    const blocks = buildSections(ranked, "all", new Set());
    const sessions = blocks.find((b) => b.section === "sessions")!;
    expect(sessions.rows).toHaveLength(ALL_VIEW_CAP + 1);
    expect(sessions.rows.at(-1)).toEqual({ kind: "more", id: "more:sessions", section: "sessions", hidden: 4 });
    expect(blocks.find((b) => b.section === "commands")!.rows.every((r) => r.kind === "item")).toBe(true);
  });

  it("shows only the chosen category, uncapped until it is large", () => {
    const blocks = buildSections(ranked, "sessions", new Set());
    expect(blocks.map((b) => b.section)).toEqual(["sessions"]);
    expect(blocks[0].rows).toHaveLength(9);
  });

  it("expands a large category in place", () => {
    const many = items("files", CATEGORY_VIEW_CAP + 7);
    const collapsed = buildSections(many, "files", new Set())[0].rows;
    expect(collapsed.at(-1)).toMatchObject({ kind: "more", hidden: 7 });
    const expanded = buildSections(many, "files", new Set<PaletteSection>(["files"]))[0].rows;
    expect(expanded).toHaveLength(CATEGORY_VIEW_CAP + 7);
  });

  it("omits empty sections", () => {
    expect(buildSections(items("commands", 2), "all", new Set()).map((b) => b.section)).toEqual(["commands"]);
  });
});

describe("selectableRows", () => {
  it("skips disabled items but keeps the more rows", () => {
    const blocks = buildSections(items("sessions", 8, 2), "all", new Set());
    const rows = selectableRows(blocks);
    expect(rows.map((r) => r.id)).toEqual(["sessions:0", "sessions:1", "more:sessions"]);
  });
});

describe("keyboard maths", () => {
  it("wraps arrow steps", () => {
    expect(stepIndex(0, -1, 5)).toBe(4);
    expect(stepIndex(4, 1, 5)).toBe(0);
    expect(stepIndex(0, 1, 0)).toBe(0);
  });

  it("pages by eight and stops at the ends", () => {
    expect(PAGE_STEP).toBe(8);
    expect(pageIndex(0, 1, 30)).toBe(8);
    expect(pageIndex(25, 1, 30)).toBe(29);
    expect(pageIndex(3, -1, 30)).toBe(0);
  });

  it("recognises an IME composition", () => {
    expect(isImeComposing({ isComposing: true })).toBe(true);
    expect(isImeComposing({ keyCode: 229 })).toBe(true);
    expect(isImeComposing({ isComposing: false, keyCode: 13 })).toBe(false);
  });
});

describe("files copy", () => {
  it("names the project and branch", () => {
    expect(filesHeader("forge", "main")).toBe("File names in forge (main)");
    expect(filesHeader("forge", null)).toBe("File names in forge");
    expect(filesHeader(null, null)).toBe("File names");
  });

  it("picks the empty state", () => {
    expect(filesEmptyState(false, "abc")).toBe("no-session");
    expect(filesEmptyState(true, "a")).toBe("short-query");
    expect(filesEmptyState(true, "ab")).toBe("idle");
  });
});
