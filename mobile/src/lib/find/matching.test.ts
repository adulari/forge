import { describe, expect, it } from "vitest";

import { matchOffsets, steppedIndex, textMatches } from "./matching";

describe("matchOffsets", () => {
  it("finds every non-overlapping hit, ignoring case unless asked", () => {
    expect(matchOffsets("Foo foo FOO", "foo", false)).toEqual([0, 4, 8]);
    expect(matchOffsets("Foo foo FOO", "foo", true)).toEqual([4]);
    expect(matchOffsets("aaaa", "aa", false)).toEqual([0, 2]);
  });

  it("reports nothing for an empty query and stops at the cap", () => {
    expect(matchOffsets("abc", "", false)).toEqual([]);
    expect(matchOffsets("a".repeat(50), "a", false, 10)).toHaveLength(10);
  });
});

describe("textMatches", () => {
  it("is a boolean matchOffsets", () => {
    expect(textMatches("Hello", "ell", false)).toBe(true);
    expect(textMatches("Hello", "ELL", true)).toBe(false);
    expect(textMatches("Hello", "", false)).toBe(false);
  });
});

describe("steppedIndex", () => {
  it("wraps at both ends", () => {
    expect(steppedIndex(2, 3, 1)).toBe(0);
    expect(steppedIndex(0, 3, -1)).toBe(2);
    expect(steppedIndex(1, 3, 1)).toBe(2);
  });

  it("enters the list from the matching end and ignores an empty one", () => {
    expect(steppedIndex(-1, 4, 1)).toBe(0);
    expect(steppedIndex(-1, 4, -1)).toBe(3);
    expect(steppedIndex(-1, 0, 1)).toBe(-1);
  });
});
