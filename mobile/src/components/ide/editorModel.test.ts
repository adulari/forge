import { describe, expect, it } from "vitest";

import { classifySaveError, externalChange, indentSelection, lineCountOf, lineOfOffset, lineWindow, scrollTopFor } from "./editorModel";

describe("indentSelection", () => {
  it("inserts spaces at a bare caret", () => {
    expect(indentSelection("ab", 1, 1, false)).toEqual({ value: "a  b", start: 3, end: 3 });
  });
  it("indents every selected line", () => {
    const r = indentSelection("a\nb\nc", 0, 3, false);
    expect(r.value).toBe("  a\n  b\nc");
    expect(r.start).toBe(2);
    expect(r.end).toBe(7);
  });
  it("outdents a line and clamps at the line start", () => {
    expect(indentSelection("  a\n b", 2, 2, true).value).toBe("a\n b");
    const r = indentSelection("    x", 1, 1, true);
    expect(r.value).toBe("  x");
    expect(r.start).toBe(0);
  });
  it("does not touch the line a selection merely ends at", () => {
    expect(indentSelection("a\nb", 0, 2, false).value).toBe("  a\nb");
  });
});

describe("lines", () => {
  it("counts lines and finds the caret line", () => {
    expect(lineCountOf("")).toBe(1);
    expect(lineCountOf("a\nb\n")).toBe(3);
    expect(lineOfOffset("a\nbb\nc", 0)).toBe(0);
    expect(lineOfOffset("a\nbb\nc", 3)).toBe(1);
    expect(lineOfOffset("a\nbb\nc", 5)).toBe(2);
  });
  it("paints only the viewport plus overscan, clamped", () => {
    expect(lineWindow(0, 360, 5000, 10)).toEqual({ first: 0, last: 30 });
    expect(lineWindow(18 * 1000, 360, 5000, 10)).toEqual({ first: 990, last: 1030 });
    expect(lineWindow(18 * 4990, 360, 5000, 10)).toEqual({ first: 4980, last: 5000 });
    expect(lineWindow(0, 360, 3, 10)).toEqual({ first: 0, last: 3 });
  });
  it("scrolls a line into view without overshooting", () => {
    expect(scrollTopFor(0, 300, 1000)).toBe(0);
    expect(scrollTopFor(999, 300, 1000)).toBe(1000 * 18 - 300);
    expect(scrollTopFor(100, 300, 1000)).toBe(100 * 18 - 100);
  });
});

describe("save outcomes", () => {
  it("names the failure", () => {
    expect(classifySaveError(Object.assign(new Error("x"), { status: 409 }))).toBe("conflict");
    expect(classifySaveError(Object.assign(new Error("x"), { status: 413 }))).toBe("too_large");
    expect(classifySaveError(new Error("offline"))).toBe("other");
  });
  it("reloads a clean file and flags a dirty one when the agent changed it", () => {
    expect(externalChange("h1", "h1", true)).toBe("none");
    expect(externalChange("h1", "h2", false)).toBe("reload");
    expect(externalChange("h1", "h2", true)).toBe("conflict");
    expect(externalChange("h1", undefined, true)).toBe("none");
  });
});
