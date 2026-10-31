import { describe, expect, it } from "vitest";

import { allowInput } from "./permissionNote";

describe("allowInput", () => {
  it("sends the reason with a deny", () => {
    expect(allowInput(false, 7, "  too risky  ")).toEqual({ kind: "allow", yes: false, seq: 7, note: "too risky" });
  });

  it("sends the instruction with an allow", () => {
    expect(allowInput(true, 3, "only in src/")).toEqual({ kind: "allow", yes: true, seq: 3, note: "only in src/" });
  });

  it("has no note key without text", () => {
    for (const note of ["", "   "]) {
      const input = allowInput(false, 1, note);
      expect(input).toEqual({ kind: "allow", yes: false, seq: 1 });
      expect("note" in input).toBe(false);
    }
  });
});
