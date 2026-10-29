import { describe, expect, it } from "vitest";

import { nextSelection, optionIndexForKey } from "./questionKeys";

describe("optionIndexForKey", () => {
  it("maps 1-9 to a zero-based row inside the option list", () => {
    expect(optionIndexForKey("1", 3)).toBe(0);
    expect(optionIndexForKey("3", 3)).toBe(2);
    expect(optionIndexForKey("9", 9)).toBe(8);
  });

  it("ignores digits past the last row and anything that is not 1-9", () => {
    expect(optionIndexForKey("4", 3)).toBeNull();
    expect(optionIndexForKey("0", 3)).toBeNull();
    expect(optionIndexForKey("a", 3)).toBeNull();
    expect(optionIndexForKey("10", 20)).toBeNull();
    expect(optionIndexForKey("Enter", 3)).toBeNull();
  });
});

describe("nextSelection", () => {
  it("clears a selected single-choice row when pressed again", () => {
    expect(nextSelection(["A"], "A", false)).toEqual([]);
  });

  it("replaces the selection when another single-choice row is pressed", () => {
    expect(nextSelection(["A"], "B", false)).toEqual(["B"]);
    expect(nextSelection([], "B", false)).toEqual(["B"]);
  });

  it("toggles multi-choice rows independently", () => {
    expect(nextSelection(["A"], "B", true)).toEqual(["A", "B"]);
    expect(nextSelection(["A", "B"], "A", true)).toEqual(["B"]);
  });
});
