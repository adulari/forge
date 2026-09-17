import { describe, expect, it } from "vitest";

import { headEllipsis } from "./pathTruncate";

describe("headEllipsis", () => {
  it("returns short paths unchanged", () => {
    expect(headEllipsis("src/app.ts")).toBe("src/app.ts");
  });

  it("always keeps the basename, even past a long directory segment (#27)", () => {
    const path =
      "/home/user/.forge/worktrees/9f1c2b3a-4d5e-6f70-8192-a3b4c5d6e7f8/android-test.txt";
    const result = headEllipsis(path);
    expect(result.endsWith("/android-test.txt")).toBe(true);
    expect(result.startsWith("…")).toBe(true);
    expect(result.length).toBeLessThanOrEqual(42);
  });

  it("falls back to a tail-keep of the basename itself when the name alone exceeds the budget", () => {
    const longName = `${"a".repeat(60)}.txt`;
    const result = headEllipsis(`/some/dir/${longName}`, 42);
    expect(result.startsWith("…")).toBe(true);
    expect(result.endsWith(".txt")).toBe(true);
    expect(result.length).toBeLessThanOrEqual(42);
  });

  it("respects a custom max", () => {
    const path = "/a/b/c/d/e/f/g/filename.txt";
    const result = headEllipsis(path, 16);
    expect(result.endsWith("/filename.txt")).toBe(true);
    expect(result.length).toBeLessThanOrEqual(16);
  });
});
