import { describe, expect, it } from "vitest";

import type { IssueRow } from "../../lib/apiIssues";
import { issueSubtitle, mergeIssues, startHint, startRequest } from "./issuesModel";

const row = (number: number, updated: string, url = `https://github.com/a/b/issues/${number}`): IssueRow => ({
  number,
  title: `t${number}`,
  url,
  state: "open",
  labels: [],
  author: "bot",
  created_at: updated,
  updated_at: updated,
  comment_count: number,
  repo: "a/b",
});

describe("mergeIssues", () => {
  it("sorts newest first, keeps one copy of a repeated issue and remembers its session", () => {
    const a = { issues: [row(1, "2026-11-01"), row(2, "2026-11-03")] };
    const b = { issues: [row(2, "2026-11-03"), row(3, "2026-11-02")] };
    const merged = mergeIssues([a, undefined, b], ["s1", "s2", "s3"]);
    expect(merged.map((i) => [i.number, i.sessionId])).toEqual([
      [2, "s1"],
      [3, "s3"],
      [1, "s1"],
    ]);
  });
});

describe("issueSubtitle", () => {
  it("counts comments and drops an empty age", () => {
    expect(issueSubtitle(row(1, "x"), "2h ago")).toBe("#1 - bot - 2h ago - 1 comment");
    expect(issueSubtitle(row(3, "x"), "")).toBe("#3 - bot - 3 comments");
  });
});

describe("startRequest", () => {
  const issue = { ...row(12, "x"), sessionId: "s9" };
  it("carries the merge method only when merging", () => {
    expect(startRequest(issue, { worktree: true, mode: "fix", method: "rebase" })).toEqual({
      session: "s9",
      number: 12,
      worktree: true,
      mode: "fix",
    });
    expect(startRequest(issue, { worktree: false, mode: "merge", method: "rebase" })).toEqual({
      session: "s9",
      number: 12,
      worktree: false,
      mode: "merge",
      method: "rebase",
    });
  });
});

describe("startHint", () => {
  it("names the worktree and the watch mode", () => {
    expect(startHint("merge", true)).toContain("in a new worktree");
    expect(startHint("merge", true)).toContain("merge when green");
    expect(startHint("off", false)).toContain("in the project checkout");
    expect(startHint("off", false)).toContain("PR Watch stays off");
  });
});
