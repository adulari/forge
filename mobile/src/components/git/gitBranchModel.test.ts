import { describe, expect, it } from "vitest";

import {
  canSelectGitBranch,
  filterGitBranches,
  gitBranchSubtitle,
  gitWorktreePathTail,
} from "./gitBranchModel";
import { type GitBranchRow } from "../../lib/api";

function row(overrides: Partial<GitBranchRow> = {}): GitBranchRow {
  return {
    name: "feature/branch-ui",
    oid: "abc1234",
    upstream: null,
    remote: false,
    current: false,
    default: false,
    worktree: null,
    ...overrides,
  };
}

describe("git branch picker model", () => {
  it("filters branch names case-insensitively", () => {
    const rows = [
      row({ name: "main" }),
      row({ name: "Feature/Mobile" }),
      row({ name: "origin/feature/web", remote: true }),
    ];
    expect(filterGitBranches(rows, " mobile ")).toEqual([rows[1]]);
    expect(filterGitBranches(rows, "FEATURE")).toHaveLength(2);
    expect(filterGitBranches(rows, "")).toBe(rows);
  });

  it("explains current, default, tracking, remote, and worktree state", () => {
    expect(
      gitBranchSubtitle(
        row({
          name: "main",
          current: true,
          default: true,
          upstream: "origin/main",
        }),
      ),
    ).toBe("current · default · tracks origin/main");
    expect(
      gitBranchSubtitle(
        row({
          name: "origin/feature/mobile",
          remote: true,
          worktree: "/repo/.forge/worktrees/child",
        }),
      ),
    ).toBe("remote · worktree · .forge/worktrees/child");
    expect(gitWorktreePathTail("C:\\repo\\other\\tree")).toBe("repo/other/tree");
  });

  it("only enables an unoccupied branch when repository actions are safe", () => {
    expect(canSelectGitBranch(row(), null, false)).toBe(true);
    expect(canSelectGitBranch(row({ current: true }), null, false)).toBe(false);
    expect(canSelectGitBranch(row({ worktree: "/repo/other" }), null, false)).toBe(false);
    expect(canSelectGitBranch(row(), "working tree dirty", false)).toBe(false);
    expect(canSelectGitBranch(row(), null, true)).toBe(false);
  });
});
