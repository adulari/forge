import { describe, expect, it } from "vitest";

import { filterByProject, projectChips } from "./fleetProjects";

const rows = [
  { cwd: "/w/forge", waiting: true },
  { cwd: "/w/forge/.forge/worktrees/abcd1234", waiting: false },
  { cwd: "/w/site", waiting: false },
];

describe("fleet projects", () => {
  it("counts a worktree under its repo and orders the busiest project first", () => {
    expect(projectChips(rows)).toEqual([
      { name: "forge", count: 2, waiting: 1 },
      { name: "site", count: 1, waiting: 0 },
    ]);
  });

  it("filters to one project, or passes everything", () => {
    expect(filterByProject(rows, "site")).toHaveLength(1);
    expect(filterByProject(rows, "forge")).toHaveLength(2);
    expect(filterByProject(rows, null)).toHaveLength(3);
  });
});
