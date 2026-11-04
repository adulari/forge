import { describe, expect, it } from "vitest";

import { currentHunkIndex, hunkSelection, lineSelection } from "./hunkSelection";

describe("currentHunkIndex", () => {
  it("follows the first visible row", () => {
    expect(currentHunkIndex([0, 10, 30], 0)).toBe(0);
    expect(currentHunkIndex([0, 10, 30], 12)).toBe(1);
    expect(currentHunkIndex([0, 10, 30], 30)).toBe(2);
    expect(currentHunkIndex([], 5)).toBe(0);
  });
});

describe("hunkSelection", () => {
  it("anchors to the additions on the new side, with the context between them", () => {
    const sel = hunkSelection({ header: "@@ -3,3 +3,4 @@", lines: [" keep", "-old", "+new", " mid", "+extra"] });
    expect(sel?.side).toBe("new");
    expect([sel?.startLine, sel?.endLine]).toEqual([4, 6]);
    expect(sel?.lines.map((l) => l.kind)).toEqual(["add", "context", "add"]);
  });
  it("falls back to the old side for a pure deletion", () => {
    const sel = hunkSelection({ header: "@@ -10,3 +10,1 @@", lines: [" a", "-b", "-c"] });
    expect(sel?.side).toBe("old");
    expect([sel?.startLine, sel?.endLine]).toEqual([11, 12]);
  });
  it("is null for a hunk with nothing changed", () => {
    expect(hunkSelection({ header: "@@ -1 +1 @@", lines: [" a", "\\ No newline at end of file"] })).toBeNull();
  });
});

describe("lineSelection", () => {
  it("uses the old side for a deleted line", () => {
    expect(lineSelection({ lineNo: 4, kind: "del", text: "x" }).side).toBe("old");
    expect(lineSelection({ lineNo: 4, kind: "add", text: "x" }).side).toBe("new");
  });
});
