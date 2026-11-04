import { describe, expect, it } from "vitest";

import { buildDiffRows, hunkTarget } from "./diffRows";

const hunks = [
  { header: "@@ -1,2 +1,2 @@", lines: [" a", "-b", "+B"] },
  { header: "@@ -10,1 +10,2 @@", lines: [" x", "+y"] },
];

describe("buildDiffRows", () => {
  it("flattens hunks and records where each starts", () => {
    const { rows, hunkStarts } = buildDiffRows(hunks, "unified");
    expect(rows.map((r) => r.kind)).toEqual(["hunk", "line", "line", "line", "hunk", "line", "line"]);
    expect(hunkStarts).toEqual([0, 4]);
  });

  it("pairs a replaced line side by side in split mode", () => {
    const { rows } = buildDiffRows(hunks, "split");
    const pairs = rows.filter((r) => r.kind === "pair");
    expect(pairs).toHaveLength(4);
    const replaced = pairs[1];
    expect(replaced.kind === "pair" && replaced.left?.text).toBe("b");
    expect(replaced.kind === "pair" && replaced.right?.text).toBe("B");
  });

  it("walks hunks without wrapping", () => {
    expect(hunkTarget([0, 4], 0, 1)).toBe(4);
    expect(hunkTarget([0, 4], 4, 1)).toBeNull();
    expect(hunkTarget([0, 4], 6, -1)).toBe(4);
    expect(hunkTarget([0, 4], 4, -1)).toBe(0);
    expect(hunkTarget([0, 4], 0, -1)).toBeNull();
    expect(hunkTarget([], 0, 1)).toBeNull();
  });

  it("handles a 5,000 line file quickly", () => {
    const lines = Array.from({ length: 5000 }, (_, i) => (i % 3 === 0 ? `+line ${i}` : ` line ${i}`));
    const started = performance.now();
    const { rows } = buildDiffRows([{ header: "@@ -1,5000 +1,5000 @@", lines }], "unified");
    const elapsed = performance.now() - started;
    expect(rows).toHaveLength(5001);
    expect(elapsed).toBeLessThan(50);
  });
});
