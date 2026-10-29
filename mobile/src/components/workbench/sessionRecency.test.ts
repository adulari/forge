import { describe, expect, it } from "vitest";

import { adjacentSession, recentAt, touchRecent } from "./sessionRecency";

describe("session recency", () => {
  it("moves a visited session to the front without duplicates", () => {
    expect(touchRecent(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
    expect(touchRecent([], "a")).toEqual(["a"]);
  });

  it("cycles the three most recent sessions with repeated presses", () => {
    const base = ["c", "b", "a", "z"];
    const known = new Set(base);
    expect(recentAt(base, known, 1)).toBe("b");
    expect(recentAt(base, known, 2)).toBe("a");
    expect(recentAt(base, known, 3)).toBe("z");
    expect(recentAt(base, known, 4)).toBe("c");
  });

  it("skips sessions that no longer exist and needs two to cycle", () => {
    expect(recentAt(["c", "gone", "a"], new Set(["c", "a"]), 1)).toBe("a");
    expect(recentAt(["c"], new Set(["c"]), 1)).toBeNull();
  });

  it("wraps next and previous around the list", () => {
    expect(adjacentSession(["a", "b", "c"], "c", 1)).toBe("a");
    expect(adjacentSession(["a", "b", "c"], "a", -1)).toBe("c");
    expect(adjacentSession(["a", "b", "c"], null, 1)).toBe("a");
    expect(adjacentSession([], "a", 1)).toBeNull();
  });
});
