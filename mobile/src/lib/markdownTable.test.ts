import { describe, expect, it } from "vitest";

import { columnWidths, fitRow, isTableDelimiter, opensTable, tableAlignments, tableCells } from "./markdownTable";

describe("markdown tables", () => {
  it("recognises delimiter rows with and without outer pipes or alignment", () => {
    expect(isTableDelimiter("|---|---|---|---|")).toBe(true);
    expect(isTableDelimiter("| :--- | ---: | :-: |")).toBe(true);
    expect(isTableDelimiter("--- | ---")).toBe(true);
    expect(isTableDelimiter("---")).toBe(false);
    expect(isTableDelimiter("| a | b |")).toBe(false);
  });

  it("opens a table only on a header followed by a delimiter", () => {
    expect(opensTable("| Test | Result |", "|---|---|")).toBe(true);
    expect(opensTable("plain text", "|---|---|")).toBe(false);
    expect(opensTable("| a | b |", undefined)).toBe(false);
  });

  it("splits cells, keeping escaped pipes and pipes inside code", () => {
    expect(tableCells("| `test_add` | `a | b` | pass |")).toEqual(["`test_add`", "`a | b`", "pass"]);
    expect(tableCells("x \\| y | z")).toEqual(["x | y", "z"]);
  });

  it("fits rows to the header width", () => {
    expect(fitRow(["a"], 3)).toEqual(["a", "", ""]);
    expect(fitRow(["a", "b", "c", "d"], 2)).toEqual(["a", "b"]);
  });
});

describe("table alignment and widths", () => {
  it("reads alignment from the delimiter row", () => {
    expect(tableAlignments("|:---|---:|:---:|---|")).toEqual(["left", "right", "center", null]);
    expect(tableAlignments("--- | ---:")).toEqual([null, "right"]);
  });

  it("sizes columns by their longest cell within bounds, ignoring markdown markers", () => {
    const widths = columnWidths([
      ["a", "name", "note"],
      ["**bold text here**", "x", "y".repeat(500)],
    ]);
    expect(widths[0]).toBeGreaterThan(widths[1]);
    expect(widths[1]).toBe(72);
    expect(widths[2]).toBe(280);
    expect(columnWidths([["[link](https://example.com/very/long/target)"]])[0]).toBeLessThan(100);
  });

  it("treats a delimiter that is still streaming in as not yet a table", () => {
    expect(opensTable("| arm | exact |", "|---")).toBe(false);
    expect(opensTable("| arm | exact |", "|---|--")).toBe(true);
  });
});
