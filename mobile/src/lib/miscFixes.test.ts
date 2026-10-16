import { describe, expect, it } from "vitest";

import { daysUntilIso } from "./anywhere/format";
import { resultFailed, summarizeToolLine } from "./toolRows";
import { fillerHistoryRows } from "./transcriptFiller";

describe("review fixes", () => {
  it("reads a numeric epoch-seconds trial deadline", () => {
    const now = 1_000_000_000_000;
    expect(daysUntilIso(String(now / 1000 + 3 * 86_400), now)).toBe(3);
    expect(daysUntilIso(new Date(now + 2 * 86_400_000).toISOString(), now)).toBe(2);
  });

  it("treats a denied permission as a failed tool result", () => {
    expect(resultFailed("\u2717 shell permission denied by policy")).toBe(true);
    expect(resultFailed("ok")).toBe(false);
  });

  it("summarizeToolLine tolerates a missing line", () => {
    expect(summarizeToolLine(undefined)).toBe("");
  });

  it("drops the TUI 'you' speaker header from the filler", () => {
    const out = fillerHistoryRows(
      [
        { kind: "user", text: "  you" },
        { kind: "user", text: "  hello" },
      ],
      false,
    );
    expect(out?.map((r) => r.content.trim())).toEqual(["hello"]);
  });
});
