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
    const hunks = [{ header: "@@ -1,5000 +1,5000 @@", lines }];
    // The fastest of several runs after a warm-up: a loaded CI runner inflates single runs,
    // while a real regression slows every run.
    buildDiffRows(hunks, "unified");
    let fastest = Infinity;
    let count = 0;
    for (let run = 0; run < 5; run += 1) {
      const started = performance.now();
      count = buildDiffRows(hunks, "unified").rows.length;
      fastest = Math.min(fastest, performance.now() - started);
    }
    expect(count).toBe(5001);
    expect(fastest).toBeLessThan(50);
  });
});
