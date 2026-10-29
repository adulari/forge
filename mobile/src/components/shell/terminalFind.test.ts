import { describe, expect, it } from "vitest";

import { type AnsiLine, DEFAULT_ANSI_STYLE } from "./ansi";
import { findTerminalHits, highlightsByLine, relocateHit, splitByHighlights } from "./terminalFind";

const line = (key: number, ...texts: string[]): AnsiLine => ({
  key,
  spans: texts.map((text) => ({ text, style: DEFAULT_ANSI_STYLE })),
});

describe("findTerminalHits", () => {
  it("finds matches across lines and across the spans of one line", () => {
    const lines = [line(4, "build ok"), line(5, "err", "or: Error"), line(6, "error")];
    const { hits, capped } = findTerminalHits(lines, "error", false);
    expect(capped).toBe(false);
    expect(hits).toEqual([
      { key: 5, start: 0, end: 5 },
      { key: 5, start: 7, end: 12 },
      { key: 6, start: 0, end: 5 },
    ]);
  });

  it("honours case and an empty query", () => {
    const lines = [line(1, "Error error")];
    expect(findTerminalHits(lines, "Error", true).hits).toHaveLength(1);
    expect(findTerminalHits(lines, "", false).hits).toEqual([]);
  });

  it("stops at the cap and says so", () => {
    const lines = [line(1, "aaaa"), line(2, "aaaa")];
    const result = findTerminalHits(lines, "a", false, 5);
    expect(result.hits).toHaveLength(5);
    expect(result.capped).toBe(true);
  });
});

describe("relocateHit", () => {
  const hits = [
    { key: 2, start: 0, end: 1 },
    { key: 5, start: 3, end: 4 },
    { key: 9, start: 0, end: 1 },
  ];

  it("keeps the same hit when its line is retained", () => {
    expect(relocateHit(hits, { key: 5, start: 3, end: 4 })).toBe(1);
  });

  it("moves to the next retained hit when its line scrolled away, and to the last when none follow", () => {
    expect(relocateHit(hits, { key: 4, start: 0, end: 1 })).toBe(1);
    expect(relocateHit(hits, { key: 99, start: 0, end: 1 })).toBe(2);
    expect(relocateHit(hits, undefined)).toBe(2);
    expect(relocateHit([], undefined)).toBe(-1);
  });
});

describe("splitByHighlights", () => {
  it("cuts a span at the match edges, using the span's offset into the line", () => {
    const marks = highlightsByLine([{ key: 1, start: 3, end: 8 }], 0).get(1);
    expect(splitByHighlights("cdefg", 2, marks)).toEqual([
      { text: "c", active: null },
      { text: "defg", active: true },
    ]);
    expect(splitByHighlights("ab", 0, marks)).toEqual([{ text: "ab", active: null }]);
  });

  it("marks only the active hit as active and keeps order", () => {
    const marks = highlightsByLine(
      [
        { key: 1, start: 0, end: 1 },
        { key: 1, start: 2, end: 3 },
      ],
      1,
    ).get(1);
    expect(splitByHighlights("a-a-", 0, marks)).toEqual([
      { text: "a", active: false },
      { text: "-", active: null },
      { text: "a", active: true },
      { text: "-", active: null },
    ]);
  });
});
