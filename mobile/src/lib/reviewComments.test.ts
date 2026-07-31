import { beforeEach, describe, expect, it } from "vitest";

import {
  addReviewComment,
  buildReviewLineSelection,
  clearReviewComments,
  formatReviewCommentsPrompt,
  getReviewComments,
  removeReviewComment,
  resetReviewCommentsForTests,
  reviewDiffRevision,
  reviewRangeLabel,
} from "./reviewComments";

describe("review comments", () => {
  beforeEach(resetReviewCommentsForTests);

  it("keeps pending comments isolated by session and removable", () => {
    const created = addReviewComment({
      sessionId: "session-a",
      source: "working-tree",
      path: "src/main.rs",
      revision: "rev-a",
      staged: false,
      side: "new",
      startLine: 12,
      endLine: 12,
      lines: [{ lineNo: 12, kind: "add", text: "let ready = true;" }],
      text: "Should this be derived?",
    });
    addReviewComment({
      sessionId: "session-b",
      source: "turn",
      path: "README.md",
      revision: "rev-b",
      staged: false,
      side: "old",
      startLine: 2,
      endLine: 3,
      lines: [],
      text: "Keep this detail.",
    });

    expect(getReviewComments("session-a")).toHaveLength(1);
    expect(getReviewComments("session-b")).toHaveLength(1);
    removeReviewComment("session-a", created.id);
    expect(getReviewComments("session-a")).toHaveLength(0);
    clearReviewComments("session-b");
    expect(getReviewComments("session-b")).toHaveLength(0);
  });

  it("formats readable, line-addressed feedback for the next prompt", () => {
    const comment = addReviewComment({
      sessionId: "session-a",
      source: "working-tree",
      path: "src/main.rs",
      revision: "rev-a",
      staged: true,
      side: "new",
      startLine: 12,
      endLine: 13,
      lines: [
        { lineNo: 12, kind: "del", text: "let ready = false;" },
        { lineNo: 13, kind: "add", text: "let ready = true;" },
      ],
      text: "Please preserve the lazy behavior.",
    });
    const prompt = formatReviewCommentsPrompt("Fix my review notes.", [comment]);

    expect(reviewRangeLabel("new", 12, 13)).toBe("new L12–L13");
    expect(prompt).toContain("Fix my review notes.");
    expect(prompt).toContain("`src/main.rs` · new L12–L13 · staged");
    expect(prompt).toContain("      12 -let ready = false;");
    expect(prompt).toContain("      13 +let ready = true;");
    expect(prompt).toContain("Please preserve the lazy behavior.");
  });

  it("builds a normalized contiguous selection from either tap direction", () => {
    const selection = buildReviewLineSelection(
      "new",
      [
        { lineNo: 8, kind: "context", text: "before" },
        { lineNo: 9, kind: "add", text: "changed" },
        { lineNo: 10, kind: "context", text: "after" },
      ],
      10,
      8,
    );
    expect(selection).toEqual({
      side: "new",
      startLine: 8,
      endLine: 10,
      lines: [
        { lineNo: 8, kind: "context", text: "before" },
        { lineNo: 9, kind: "add", text: "changed" },
        { lineNo: 10, kind: "context", text: "after" },
      ],
    });
  });

  it("fingerprints the exact patch so stale line markers cannot drift", () => {
    const first = reviewDiffRevision("src/main.rs", [
      { header: "@@ -1 +1 @@", lines: ["-false", "+true"] },
    ]);
    const same = reviewDiffRevision("src/main.rs", [
      { header: "@@ -1 +1 @@", lines: ["-false", "+true"] },
    ]);
    const changed = reviewDiffRevision("src/main.rs", [
      { header: "@@ -1 +1 @@", lines: ["-false", "+maybe"] },
    ]);
    expect(first).toBe(same);
    expect(first).not.toBe(changed);
  });

  it("can send annotations without additional composer text", () => {
    const comment = addReviewComment({
      sessionId: "session-a",
      source: "turn",
      path: "docs/plan.md",
      revision: "rev-c",
      staged: false,
      side: "old",
      startLine: 4,
      endLine: 4,
      lines: [],
      text: "This requirement is still needed.",
    });
    expect(formatReviewCommentsPrompt("", [comment])).toMatch(
      /^Please address the following review feedback\./,
    );
  });
});
