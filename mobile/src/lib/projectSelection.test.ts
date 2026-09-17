import { describe, expect, it } from "vitest";

import { isKnownGitRepo, isLoopbackServer, lastProjectStorageKey, projectChoices, projectName } from "./projectSelection";

describe("project selection", () => {
  it("uses one server-scoped remembered-project key", () => {
    expect(lastProjectStorageKey("srv_123")).toBe("forge.lastProject.srv_123");
  });

  it("recognizes local desktop daemons without treating remote hosts as local", () => {
    expect(isLoopbackServer("http://127.0.0.1:7420/token")).toBe(true);
    expect(isLoopbackServer("http://localhost:7420/token")).toBe(true);
    expect(isLoopbackServer("https://forge.example.com/token")).toBe(false);
  });

  it("deduplicates the default from recent projects and names cross-platform paths", () => {
    const choices = projectChoices("/work/forge", [
      { path: "/work/forge", name: "forge", is_git_repo: true, last_activity: 2 },
      { path: "/work/helm", name: "helm", is_git_repo: true, last_activity: 1 },
    ]);
    expect(choices.map((choice) => choice.path)).toEqual(["/work/forge", "/work/helm"]);
    expect(projectName("C:\\Users\\me\\forge\\")).toBe("forge");
  });

  it("reads the default project's real is_git_repo from roots instead of assuming true", () => {
    // The daemon's own cwd is resolved as roots[0] (serve_projects.rs resolve_project_roots) —
    // never in `recent`, which dedupes it out. A plain home directory is the common case.
    const choices = projectChoices("/home/floris", [], [
      { path: "/home/floris", name: "floris", is_git_repo: false, last_activity: null },
    ]);
    expect(choices[0]).toMatchObject({ path: "/home/floris", is_git_repo: false });
  });

  it("falls back to is_git_repo: false for a default cwd absent from roots, not true", () => {
    const choices = projectChoices("/home/floris", []);
    expect(choices[0]).toMatchObject({ path: "/home/floris", is_git_repo: false });
  });
});

describe("isKnownGitRepo", () => {
  const catalog = {
    recent: [{ path: "/work/helm", name: "helm", is_git_repo: true, last_activity: 1 }],
    roots: [{ path: "/home/floris", name: "floris", is_git_repo: false, last_activity: null }],
  };

  it("finds the flag in roots (the daemon's own cwd)", () => {
    expect(isKnownGitRepo("/home/floris", catalog)).toBe(false);
  });

  it("finds the flag in recent", () => {
    expect(isKnownGitRepo("/work/helm", catalog)).toBe(true);
  });

  it("is null for a path the catalog never mentioned, or with no catalog yet", () => {
    expect(isKnownGitRepo("/some/manual/path", catalog)).toBeNull();
    expect(isKnownGitRepo("/home/floris", null)).toBeNull();
    expect(isKnownGitRepo("", catalog)).toBeNull();
  });
});
