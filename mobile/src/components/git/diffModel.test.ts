import { describe, expect, it } from "vitest";

import {
  diffTextSegments,
  middleTruncate,
  parseHunkStart,
  toSplitRows,
  toUnifiedRows,
} from "./diffModel";

const hunk = {
  header: "@@ -62,6 +62,7 @@ def sweep(windows, targets):",
  lines: [
    " def sweep(windows, targets):",
    "-    lookback = 21",
    "-    vol_target = 0.12",
    "+    lookback = tuned.lookback",
    "+    vol_target = tuned.vol_target",
    "+    seed = tuned.seed",
    "     for w, t in grid:",
  ],
};

describe("git review diff model", () => {
  it("parses hunk starts, falling back rather than throwing", () => {
    expect(parseHunkStart(hunk.header)).toEqual({ oldStart: 62, newStart: 62 });
    expect(parseHunkStart("@@ -1 +1 @@")).toEqual({ oldStart: 1, newStart: 1 });
    expect(parseHunkStart("not a header")).toEqual({ oldStart: 1, newStart: 1 });
  });

  it("split pairs a replacement block and spills the extra addition one-sided", () => {
    const rows = toSplitRows([hunk]);
    expect(rows[0]).toMatchObject({ kind: "hunk" });
    const pairs = rows.filter((row) => row.kind === "pair");

    expect(pairs[0]).toMatchObject({
      left: { lineNo: 62, kind: "context" },
      right: { lineNo: 62, kind: "context" },
    });
    expect(pairs[1]).toMatchObject({
      left: { lineNo: 63, text: "    lookback = 21", kind: "del" },
      right: { lineNo: 63, text: "    lookback = tuned.lookback", kind: "add" },
    });
    // Third addition has no counterpart to delete — one-sided, not paired with the next context.
    expect(pairs[3]).toMatchObject({ left: null, right: { lineNo: 65, kind: "add" } });
    // Context after the block resumes on both sides at each side's own number.
    expect(pairs[4]).toMatchObject({
      left: { lineNo: 65, kind: "context" },
      right: { lineNo: 66, kind: "context" },
    });
  });

  it("unified numbers each line on its own side", () => {
    const cells = toUnifiedRows([hunk]).flatMap((row) => (row.kind === "line" ? [row.cell] : []));
    expect(cells.map((cell) => [cell.kind, cell.lineNo])).toEqual([
      ["context", 62],
      ["del", 63],
      ["del", 64],
      ["add", 63],
      ["add", 64],
      ["add", 65],
      ["context", 66],
    ]);
  });

  it("marks only changed tokens inside paired replacement lines", () => {
    const segments = diffTextSegments(
      " const timeout = config.shortTimeout;",
      " const timeout = config.longTimeout;",
    );
    expect(segments.oldSegments).toEqual([
      { text: " const timeout = config.", changed: false },
      { text: "shortTimeout", changed: true },
      { text: ";", changed: false },
    ]);
    expect(segments.newSegments).toEqual([
      { text: " const timeout = config.", changed: false },
      { text: "longTimeout", changed: true },
      { text: ";", changed: false },
    ]);

    const split = toSplitRows([
      {
        header: "@@ -1 +1 @@",
        lines: ["- const ready = false;", "+ const ready = true;"],
      },
    ]);
    const pair = split.find((row) => row.kind === "pair");
    expect(pair?.kind === "pair" ? pair.left?.segments : null).toContainEqual({
      text: "false",
      changed: true,
    });
    const unified = toUnifiedRows([
      {
        header: "@@ -1 +1 @@",
        lines: ["- const ready = false;", "+ const ready = true;"],
      },
    ]);
    const addition = unified.find((row) => row.kind === "line" && row.cell.kind === "add");
    expect(addition?.kind === "line" ? addition.cell.segments : null).toContainEqual({
      text: "true",
      changed: true,
    });
  });

  it("middle-truncates keeping the distinguishing tail", () => {
    expect(middleTruncate("short.py", 20)).toBe("short.py");
    expect(middleTruncate("crates/forge-cli/src/serve_git.rs", 20)).toBe("crates/fo…rve_git.rs");
  });
});
