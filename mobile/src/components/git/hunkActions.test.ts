import { describe, expect, it } from "vitest";

import { ApiError } from "../../lib/api";
import { buildHunkPatch, classifyHunkError, hunkActionsFor } from "./hunkActions";

const plain = { binary: false, kind: "modified" as const, skipped_lines: 0, orig_path: null };

describe("hunkActionsFor", () => {
  it("offers stage and revert on unstaged hunks, unstage on staged ones", () => {
    expect(hunkActionsFor("unstaged", plain, false).map((a) => a.action)).toEqual(["stage", "revert"]);
    expect(hunkActionsFor("staged", plain, false).map((a) => a.action)).toEqual(["unstage"]);
    expect(hunkActionsFor("untracked", { ...plain, kind: "created" }, false).map((a) => a.action)).toEqual(["stage"]);
  });
  it("marks only revert as needing confirmation", () => {
    const specs = hunkActionsFor("unstaged", plain, false);
    expect(specs.map((a) => a.destructive)).toEqual([false, true]);
  });
  it("offers nothing read-only, binary, renamed or cut short", () => {
    expect(hunkActionsFor("unstaged", plain, true)).toEqual([]);
    expect(hunkActionsFor("unstaged", { ...plain, binary: true }, false)).toEqual([]);
    expect(hunkActionsFor("staged", { ...plain, kind: "renamed", orig_path: "a" }, false)).toEqual([]);
    expect(hunkActionsFor("unstaged", { ...plain, skipped_lines: 5 }, false)).toEqual([]);
  });
});

describe("buildHunkPatch", () => {
  const hunk = { header: "@@ -1,2 +1,2 @@", lines: [" keep", "-old", "+new"] };
  it("builds a modify patch", () => {
    expect(buildHunkPatch({ path: "src/a.ts", kind: "modified" }, hunk)).toBe(
      "--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n",
    );
  });
  it("passes the no-newline marker through", () => {
    const cut = { header: "@@ -1 +1 @@", lines: ["-a", "\\ No newline at end of file", "+b", "\\ No newline at end of file"] };
    expect(buildHunkPatch({ path: "f", kind: "modified" }, cut)).toBe(
      "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n\\ No newline at end of file\n",
    );
  });
  it("uses /dev/null for created and deleted files", () => {
    expect(buildHunkPatch({ path: "n", kind: "created" }, hunk)).toContain("--- /dev/null\n+++ b/n\n");
    expect(buildHunkPatch({ path: "n", kind: "deleted" }, hunk)).toContain("--- a/n\n+++ /dev/null\n");
  });
});

describe("classifyHunkError", () => {
  it("tells a stale patch from a busy session and from other failures", () => {
    expect(classifyHunkError(new ApiError(409, "x", { code: "stale_patch" })).kind).toBe("stale");
    expect(classifyHunkError(new ApiError(409, "busy", { code: "session_busy" })).kind).toBe("busy");
    expect(classifyHunkError(new ApiError(400, "bad")).kind).toBe("error");
    expect(classifyHunkError(new Error("boom")).message).toBe("boom");
  });
});
