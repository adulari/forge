import { describe, expect, it } from "vitest";

import { parseWorktreePath, rememberableProject, repoRootOf, worktreeLabel } from "./worktreePaths";

const GLOBAL = "/home/u/.local/share/forge/worktrees/forge-3fa9c1d2/calm-otter-3f";

describe("parseWorktreePath", () => {
  it("reads the project-root shape with its repo root", () => {
    expect(parseWorktreePath("/home/u/forge/.forge/worktrees/abc123def")).toEqual({
      repoName: "forge",
      repoRoot: "/home/u/forge",
      name: "abc123def",
    });
  });

  it("reads the global-root shape, which carries only the repo name", () => {
    expect(parseWorktreePath(GLOBAL)).toEqual({ repoName: "forge", repoRoot: null, name: "calm-otter-3f" });
    expect(parseWorktreePath(`${GLOBAL}/src/lib`)?.name).toBe("calm-otter-3f");
  });

  it("keeps hyphens inside the repo name and handles macOS data dirs", () => {
    const path = "/Users/u/Library/Application Support/forge/worktrees/vectra-bot-0a1b2c3d/swift-fox-9a";
    expect(parseWorktreePath(path)).toMatchObject({ repoName: "vectra-bot", name: "swift-fox-9a" });
  });

  it("ignores ordinary paths", () => {
    expect(parseWorktreePath("/home/u/forge")).toBeNull();
    expect(parseWorktreePath("/home/u/worktrees/notes/a")).toBeNull();
  });
});

describe("repoRootOf", () => {
  it("strips a project-root worktree and leaves everything else alone", () => {
    expect(repoRootOf("/home/u/forge/.forge/worktrees/abc")).toBe("/home/u/forge");
    expect(repoRootOf("/home/u/forge")).toBe("/home/u/forge");
    expect(repoRootOf(GLOBAL)).toBe(GLOBAL);
  });
});

describe("worktreeLabel", () => {
  it("shortens ids but not friendly names", () => {
    expect(worktreeLabel({ repoName: "forge", repoRoot: null, name: "0123456789abcdef" })).toBe("forge · wt 01234567");
    expect(worktreeLabel({ repoName: "forge", repoRoot: null, name: "calm-otter-3f" })).toBe("forge · wt calm-otter-3f");
  });
});

describe("rememberableProject", () => {
  it("uses the repo root, or the requested folder for a global worktree", () => {
    expect(rememberableProject("/r/.forge/worktrees/a", "/r")).toBe("/r");
    expect(rememberableProject(GLOBAL, "/home/u/forge")).toBe("/home/u/forge");
    expect(rememberableProject(GLOBAL, "")).toBe(GLOBAL);
    expect(rememberableProject("/plain", "/other")).toBe("/plain");
  });
});
