import { describe, expect, it } from "vitest";

import {
  cloneNameFromUrl,
  cloneUrlProblem,
  filterGhRepos,
  manageChanges,
  offersGitInit,
  repairSummary,
} from "./projectOnboarding";

describe("cloneUrlProblem", () => {
  it("accepts https and ssh forms", () => {
    expect(cloneUrlProblem("https://github.com/octocat/Hello-World.git")).toBeNull();
    expect(cloneUrlProblem("git@github.com:octocat/Hello-World.git")).toBeNull();
    expect(cloneUrlProblem("ssh://git@host/x/y.git")).toBeNull();
  });
  it("rejects empty, spaced and blocked schemes", () => {
    expect(cloneUrlProblem("  ")).toMatch(/Paste/);
    expect(cloneUrlProblem("https://a/b c")).toMatch(/spaces/);
    expect(cloneUrlProblem("ext::sh -c touch /tmp/x")).toMatch(/not allowed/);
    expect(cloneUrlProblem("file:///tmp/repo")).toMatch(/not allowed/);
    expect(cloneUrlProblem("just-a-word")).toMatch(/https/);
  });
});

describe("cloneNameFromUrl", () => {
  it("derives the folder name", () => {
    expect(cloneNameFromUrl("https://github.com/octocat/Hello-World.git")).toBe("Hello-World");
    expect(cloneNameFromUrl("git@github.com:octocat/spoon-knife")).toBe("spoon-knife");
    expect(cloneNameFromUrl("https://host/a/b/?x=1")).toBe("b");
  });
});

describe("filterGhRepos", () => {
  const repos = [
    { name_with_owner: "me/forge", url: "u1", description: "Coding harness", private: false },
    { name_with_owner: "me/notes", url: "u2", description: "", private: true },
  ];
  it("matches owner/name and description, case-insensitively", () => {
    expect(filterGhRepos(repos, "").length).toBe(2);
    expect(filterGhRepos(repos, "HARNESS").map((r) => r.url)).toEqual(["u1"]);
    expect(filterGhRepos(repos, "notes").map((r) => r.url)).toEqual(["u2"]);
    expect(filterGhRepos(repos, "zzz")).toEqual([]);
  });
});

describe("manageChanges", () => {
  const project = { path: "/home/u/forge", name: "forge" };
  it("returns null when nothing changed", () => {
    expect(manageChanges(project, { name: "forge", relocate: "" })).toBeNull();
  });
  it("sends a custom name, and clears it when reset to the folder name", () => {
    expect(manageChanges(project, { name: " Forge App ", relocate: "" })).toEqual({ path: project.path, name: "Forge App" });
    expect(manageChanges({ ...project, name: "Custom" }, { name: "forge", relocate: "" })).toEqual({ path: project.path, name: "" });
  });
  it("sends relocation changes, including clearing", () => {
    expect(manageChanges(project, { name: "forge", relocate: "/mnt/forge" })).toEqual({ path: project.path, relocate_to: "/mnt/forge" });
    expect(manageChanges(project, { name: "forge", relocate: "" }, "/mnt/forge")).toEqual({ path: project.path, relocate_to: "" });
  });
});

describe("repairSummary / offersGitInit", () => {
  it("words the repair result", () => {
    expect(repairSummary({ merged: 0, groups: [] })).toBe("No duplicate projects found.");
    expect(repairSummary({ merged: 3, groups: [{ keep: "a", duplicates: ["b"] }] })).toBe("Merged 3 duplicate entries into 1 project.");
  });
  it("offers init only for a known non-repo", () => {
    expect(offersGitInit(false, "/x")).toBe(true);
    expect(offersGitInit(null, "/x")).toBe(false);
    expect(offersGitInit(true, "/x")).toBe(false);
    expect(offersGitInit(false, " ")).toBe(false);
  });
});
