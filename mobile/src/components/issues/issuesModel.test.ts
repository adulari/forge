import { describe, expect, it } from "vitest";

import type { IssueDrain, IssueRow } from "../../lib/apiIssues";
import {
  drainCoversRepo,
  drainHint,
  drainRequest,
  drainStatusByNumber,
  issueSubtitle,
  mergeIssues,
  pickDrain,
  startHint,
  startRequest,
  toggleSelection,
  type IssueEntry,
} from "./issuesModel";

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

const entry = (number: number, repo = "a/b", sessionId = "s1"): IssueEntry => ({
  ...row(number, "2026-11-01"),
  repo,
  sessionId,
  url: `https://github.com/${repo}/issues/${number}`,
});

describe("toggleSelection", () => {
  it("adds, removes, and restarts the selection when another repository is picked", () => {
    let picked: IssueEntry[] = [];
    picked = toggleSelection(picked, entry(1));
    picked = toggleSelection(picked, entry(2));
    expect(picked.map((p) => p.number)).toEqual([1, 2]);
    picked = toggleSelection(picked, entry(1));
    expect(picked.map((p) => p.number)).toEqual([2]);
    picked = toggleSelection(picked, entry(9, "c/d", "s2"));
    expect(picked.map((p) => [p.repo, p.number])).toEqual([["c/d", 9]]);
  });

  it("stops at the daemon's cap", () => {
    let picked: IssueEntry[] = [];
    for (let n = 1; n <= 35; n++) picked = toggleSelection(picked, entry(n));
    expect(picked).toHaveLength(30);
  });
});

describe("drainRequest", () => {
  const picked = [entry(12, "a/b", "s7"), entry(15, "a/b", "s7")];
  const base = { parallel: 2, budget: "", worktree: true, mode: "fix", method: "squash" } as const;

  it("names the issues in order through the first one's session and omits an empty budget", () => {
    expect(drainRequest(picked, base)).toEqual({
      session: "s7",
      numbers: [12, 15],
      parallel: 2,
      worktree: true,
      mode: "fix",
    });
  });

  it("carries the budget, clamps parallelism, and sends the merge method only when merging", () => {
    expect(drainRequest(picked, { ...base, parallel: 40, budget: "claude:30%w", mode: "merge", method: "rebase" })).toEqual({
      session: "s7",
      numbers: [12, 15],
      parallel: 8,
      worktree: true,
      mode: "merge",
      budget: "claude:30%w",
      method: "rebase",
    });
    expect(drainRequest(picked, { ...base, parallel: 0 }).parallel).toBe(1);
  });
});

describe("drainHint", () => {
  it("says how many run at once and what the envelope does", () => {
    expect(drainHint(3, 2, "claude:30%w")).toContain("3 issues, 2 at a time");
    expect(drainHint(3, 2, "claude:30%w")).toContain("30% weekly is spent");
    expect(drainHint(1, 1, "")).toContain("1 issue, 1 at a time");
    expect(drainHint(1, 1, "")).toContain("no spend limit");
  });
});

describe("pickDrain", () => {
  const drain = (id: string, status: IssueDrain["status"]): IssueDrain => ({
    id,
    cwd: "/p",
    status,
    parallel: 2,
    budget: null,
    paused_reason: null,
    created_at: 0,
    updated_at: 0,
    line: "Drain: 1 started",
    envelope: null,
    items: [{ number: 12, status: "running", session_id: "s", title: null, pr_url: null, outcome: null }],
  });

  it("prefers a live drain over a newer finished one", () => {
    expect(pickDrain([drain("new", "finished"), drain("old", "paused")])?.id).toBe("old");
    expect(pickDrain([drain("new", "finished"), drain("old", "cancelled")])?.id).toBe("new");
    expect(pickDrain([])).toBeNull();
    expect(pickDrain(undefined)).toBeNull();
  });

  it("maps issue numbers to item status", () => {
    expect(drainStatusByNumber(drain("a", "active")).get(12)).toBe("running");
    expect(drainStatusByNumber(null).size).toBe(0);
  });
});

describe("drainCoversRepo", () => {
  it("matches the checkout folder to the repository name", () => {
    const d = { cwd: "/home/f/Repos/forge/" } as IssueDrain;
    expect(drainCoversRepo(d, "adulari/forge")).toBe(true);
    expect(drainCoversRepo(d, "adulari/other")).toBe(false);
    expect(drainCoversRepo(null, "adulari/forge")).toBe(false);
    expect(drainCoversRepo(d, "")).toBe(false);
  });
});
