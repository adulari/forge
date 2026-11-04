import { beforeEach, describe, expect, it } from "vitest";

import { addReviewComment, getReviewComments, clearReviewComments, resetReviewCommentsForTests, type ReviewComment } from "../../lib/reviewComments";
import { buildReviewPrompt, buildSingleCommentPrompt, draftLabel, explainPrompt, groupDraftByFile } from "./reviewDraft";

function add(session: string, path: string, text: string, line = 3): ReviewComment {
  return addReviewComment({
    sessionId: session,
    source: "working-tree",
    path,
    revision: "r",
    staged: false,
    side: "new",
    startLine: line,
    endLine: line,
    lines: [{ lineNo: line, kind: "add", text: "let x = 1;" }],
    text,
  });
}

beforeEach(resetReviewCommentsForTests);

describe("review draft batching", () => {
  it("collects comments per agent and sends them as one prompt", () => {
    add("s1", "a.rs", "rename this");
    add("s1", "b.rs", "add a test", 9);
    add("s2", "c.rs", "other agent");
    const mine = getReviewComments("s1");
    expect(draftLabel(mine.length)).toBe("2 comments");
    const prompt = buildReviewPrompt(mine);
    expect(prompt).toContain("Review annotations:");
    expect(prompt).toContain("1. `a.rs`");
    expect(prompt).toContain("2. `b.rs`");
    expect(prompt).not.toContain("c.rs");
    clearReviewComments("s1");
    expect(getReviewComments("s1")).toHaveLength(0);
    expect(getReviewComments("s2")).toHaveLength(1);
  });
  it("sends a single comment on its own", () => {
    const one = add("s1", "a.rs", "just this");
    const prompt = buildSingleCommentPrompt(one);
    expect(prompt).toContain("just this");
    expect(prompt.match(/^\d+\. /gm)).toHaveLength(1);
  });
  it("groups by file in first-seen order", () => {
    const comments = [add("s", "b.rs", "1"), add("s", "a.rs", "2"), add("s", "b.rs", "3")];
    expect(groupDraftByFile(comments).map((g) => [g.path, g.comments.length])).toEqual([["b.rs", 2], ["a.rs", 1]]);
  });
  it("labels one comment in the singular", () => {
    expect(draftLabel(1)).toBe("1 comment");
  });
});

describe("explainPrompt", () => {
  it("is a /btw side question with the hunk, capped", () => {
    const text = explainPrompt("a.rs", "@@ -1 +1 @@", Array.from({ length: 100 }, (_, i) => `+l${i}`));
    expect(text.startsWith("/btw ")).toBe(true);
    expect(text).toContain("40 more lines");
    expect(text).not.toContain("+l99");
  });
});
