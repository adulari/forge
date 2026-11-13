import { describe, expect, it } from "vitest";

import { allowInput } from "./permissionNote";

describe("allowInput", () => {
  it("sends the reason with a deny", () => {
    expect(allowInput(false, 7, "  too risky  ", true)).toEqual({ kind: "allow", yes: false, seq: 7, note: "too risky" });
  });

  it("sends the instruction with an allow", () => {
    expect(allowInput(true, 3, "only in src/", true)).toEqual({ kind: "allow", yes: true, seq: 3, note: "only in src/" });
  });

  it("has no note key without text", () => {
    for (const note of ["", "   "]) {
      const input = allowInput(false, 1, note, true);
      expect(input).toEqual({ kind: "allow", yes: false, seq: 1 });
      expect("note" in input).toBe(false);
    }
  });

  it("never sends a note typed before the field was hidden", () => {
    const input = allowInput(false, 2, "stale text", false);
    expect(input).toEqual({ kind: "allow", yes: false, seq: 2 });
    expect("note" in input).toBe(false);
  });

  it("sends the session scope only with an allow", () => {
    expect(allowInput(true, 4, "", false, "session")).toEqual({ kind: "allow", yes: true, seq: 4, scope: "session" });
    expect(allowInput(false, 4, "", false, "session")).toEqual({ kind: "allow", yes: false, seq: 4 });
    expect("scope" in allowInput(true, 4, "", false)).toBe(false);
  });
});

describe("allowInput rule scope", () => {
  it("accepts the offered rule only with an allow", () => {
    expect(allowInput(true, 5, "", false, "rule")).toEqual({ kind: "allow", yes: true, seq: 5, scope: "rule" });
    expect("scope" in allowInput(false, 5, "", false, "rule")).toBe(false);
  });
});
