import { describe, expect, it } from "vitest";

import {
  grewAtOldestEnd,
  offsetFromNewest,
  scrollTopAfterResize,
  scrollTopFor,
  timelineShape,
} from "./newestAnchoredScroll";

describe("newest-anchored scroll", () => {
  it("reports 0 at the bottom and grows toward older rows", () => {
    expect(offsetFromNewest({ scrollTop: 436, contentHeight: 1000, layoutHeight: 564 })).toBe(0);
    expect(offsetFromNewest({ scrollTop: 0, contentHeight: 1000, layoutHeight: 564 })).toBe(436);
    expect(offsetFromNewest({ scrollTop: 0, contentHeight: 300, layoutHeight: 564 })).toBe(0);
  });

  it("round-trips an offset through scrollTopFor", () => {
    expect(scrollTopFor(100, 1000, 564)).toBe(336);
    expect(scrollTopFor(0, 1000, 564)).toBe(436);
    expect(scrollTopFor(900, 1000, 564)).toBe(0);
  });

  it("detects a page landing at the oldest end but not a new or streaming newest row", () => {
    const base = timelineShape(["c", "b", "a"]);
    expect(grewAtOldestEnd(base, timelineShape(["c", "b", "a", "z", "y"]))).toBe(true);
    expect(grewAtOldestEnd(base, timelineShape(["c", "b", "x"]))).toBe(true);
    expect(grewAtOldestEnd(base, timelineShape(["d", "c", "b", "a"]))).toBe(false);
    expect(grewAtOldestEnd(base, timelineShape(["c", "b", "a"]))).toBe(false);
    expect(grewAtOldestEnd(null, base)).toBe(false);
  });

  it("keeps a pinned reader on the newest row as content grows", () => {
    expect(scrollTopAfterResize({ fromNewest: 0, contentHeight: 1200, layoutHeight: 564, topGrowth: false })).toBe(636);
  });

  it("keeps the visible rows still when an older page lands above a reader", () => {
    expect(scrollTopAfterResize({ fromNewest: 300, contentHeight: 2000, layoutHeight: 564, topGrowth: true })).toBe(1136);
  });

  it("leaves a reader who scrolled up alone while the newest row grows", () => {
    expect(scrollTopAfterResize({ fromNewest: 300, contentHeight: 1200, layoutHeight: 564, topGrowth: false })).toBeNull();
  });
});
