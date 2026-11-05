import { describe, expect, it } from "vitest";

import type { GitDiffFile } from "../../lib/api";
import { buildHunkPatch } from "../git/hunkActions";
import { reverseHunk, reverseHunkHeader, undoRequests, undoStepForHunk, undoStepsForFile } from "./hunkUndo";

const hunk = { header: "@@ -3,3 +3,4 @@ fn main()", lines: [" keep", "-old", "+new", "+extra", " tail"] };
const file: GitDiffFile = {
  path: "src/a.rs",
  kind: "modified",
  orig_path: null,
  binary: false,
  adds: 2,
  dels: 1,
  hunks: [hunk, { header: "@@ -20 +21 @@", lines: ["-x", "+y"] }],
  skipped_lines: 0,
};

describe("reverseHunkHeader", () => {
  it("swaps old and new ranges and keeps the context", () => {
    expect(reverseHunkHeader(hunk.header)).toBe("@@ -3,4 +3,3 @@ fn main()");
  });
  it("keeps single-line ranges without counts", () => {
    expect(reverseHunkHeader("@@ -20 +21 @@")).toBe("@@ -21 +20 @@");
  });
  it("leaves an unparseable header alone", () => {
    expect(reverseHunkHeader("not a header")).toBe("not a header");
  });
});

describe("reverseHunk", () => {
  it("swaps + and - and is its own inverse", () => {
    const reversed = reverseHunk(hunk);
    expect(reversed.lines).toEqual([" keep", "+old", "-new", "-extra", " tail"]);
    expect(reverseHunk(reversed)).toEqual(hunk);
  });
  it("does not touch no-newline markers", () => {
    expect(reverseHunk({ header: "@@ -1 +1 @@", lines: ["-a", "\\ No newline at end of file", "+b"] }).lines).toEqual([
      "+a",
      "\\ No newline at end of file",
      "-b",
    ]);
  });
});

describe("undoStepForHunk", () => {
  it("builds a patch for the same path whose header matches its hunk line", () => {
    const step = undoStepForHunk(file, hunk);
    expect(step.patch.startsWith("--- a/src/a.rs\n+++ b/src/a.rs\n")).toBe(true);
    expect(step.patch.split("\n")).toContain(step.hunk_header);
    expect(step.patch).toBe(buildHunkPatch({ path: "src/a.rs", kind: "modified" }, reverseHunk(hunk)));
  });
});

describe("undoStepsForFile", () => {
  it("returns one step per hunk, in order", () => {
    const steps = undoStepsForFile(file);
    expect(steps.map((s) => s.hunk_header)).toEqual(["@@ -3,4 +3,3 @@ fn main()", "@@ -21 +20 @@"]);
  });
  it("refuses files a patch cannot rebuild", () => {
    expect(undoStepsForFile({ ...file, kind: "created" })).toEqual([]);
    expect(undoStepsForFile({ ...file, skipped_lines: 3 })).toEqual([]);
    expect(undoStepsForFile({ ...file, binary: true })).toEqual([]);
    expect(undoStepsForFile({ ...file, orig_path: "b.rs" })).toEqual([]);
  });
  it("posts every step as a revert on the working tree", () => {
    const requests = undoRequests("s1", file.path, undoStepsForFile(file));
    expect(requests.every((r) => r.action === "revert" && r.session === "s1" && r.path === file.path)).toBe(true);
    expect(requests).toHaveLength(2);
  });
});
