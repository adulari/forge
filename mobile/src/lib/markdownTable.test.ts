import { describe, expect, it } from "vitest";

import { fitRow, isTableDelimiter, opensTable, tableCells } from "./markdownTable";

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
