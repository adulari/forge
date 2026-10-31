import { describe, expect, it } from "vitest";

import type { SessionRow } from "./api";
import { buildProjectGroups, diffLabel, failedTurn, filterByWorkspace, projectAccent, projectName, sortSessions, splitDone, walkNodes, type ScopedSession } from "./sidebarModel";

function entry(id: string, over: Partial<SessionRow> = {}, serverId = "s1"): ScopedSession {
  return {
    serverId,
    row: {
      id,
      title: id,
      cwd: "/home/u/forge",
      worktree: null,
      busy: false,
      waiting: false,
      cost_usd: 0,
      context_tokens: 0,
      context_limit: null,
      model: "claude-cli::sonnet",
      permission_mode: "default",
      created_at: 100,
      last_activity: 100,
      ...over,
    },
  };
}

describe("projectName", () => {
  it("uses the checkout's folder and folds a worktree into its repo", () => {
    expect(projectName("/home/u/Forge")).toBe("forge");
    expect(projectName("/home/u/forge/.forge/worktrees/abc123def")).toBe("forge");
    expect(projectName("/")).toBe("~");
  });

  it("folds a global-root worktree into its repo by name", () => {
    expect(projectName("/home/u/.local/share/forge/worktrees/Forge-3fa9c1d2/calm-otter-3f")).toBe("forge");
  });
});

describe("sortSessions", () => {
  it("keeps sessions that need a decision first, then orders by the chosen key", () => {
    const rows = [
      entry("old-active", { created_at: 1, last_activity: 900 }),
      entry("new", { created_at: 500, last_activity: 600 }),
      entry("waiting", { created_at: 2, last_activity: 1, waiting: true }),
    ];
    expect(sortSessions(rows, "updated").map((e) => e.row.id)).toEqual(["waiting", "old-active", "new"]);
    expect(sortSessions(rows, "created").map((e) => e.row.id)).toEqual(["waiting", "new", "old-active"]);
  });
});

describe("buildProjectGroups", () => {
  it("nests a fork under its parent and counts both in the project", () => {
    const groups = buildProjectGroups(
      [entry("fork", { parent_id: "root", last_activity: 300 }), entry("root", { last_activity: 200 })],
      "updated",
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].nodes.map((n) => n.entry.row.id)).toEqual(["root"]);
    expect(groups[0].nodes[0].children.map((n) => n.entry.row.id)).toEqual(["fork"]);
    expect(groups[0].sessionCount).toBe(2);
    expect([...walkNodes(groups[0].nodes)].map((w) => w.depth)).toEqual([0, 1]);
  });

  it("leaves an orphan, a self-parent and a cycle at the top level", () => {
    const groups = buildProjectGroups(
      [
        entry("orphan", { parent_id: "missing" }),
        entry("self", { parent_id: "self" }),
        entry("a", { parent_id: "b" }),
        entry("b", { parent_id: "a" }),
      ],
      "updated",
    );
    const top = groups[0].nodes.map((n) => n.entry.row.id).sort();
    expect(top.length + groups[0].nodes.reduce((n, g) => n + g.children.length, 0)).toBe(4);
    expect(top).toContain("orphan");
    expect(top).toContain("self");
  });

  it("does not nest across servers", () => {
    const groups = buildProjectGroups([entry("p", {}, "s1"), entry("c", { parent_id: "p" }, "s2")], "updated");
    expect(groups).toHaveLength(2);
  });

  it("counts what needs attention inside a group", () => {
    const groups = buildProjectGroups(
      [
        entry("w", { waiting: true }),
        entry("b", { busy: true }),
        entry("f", { last_turn_outcome: "failed" }),
        entry("ok", { last_turn_outcome: "success" }),
      ],
      "updated",
    );
    expect([groups[0].waitingCount, groups[0].busyCount, groups[0].failedCount]).toEqual([1, 1, 1]);
  });

  it("groups a global-root worktree under its repo and adopts the repo's real root", () => {
    const groups = buildProjectGroups(
      [
        entry("g", { cwd: "/home/u/.local/share/forge/worktrees/forge-3fa9c1d2/calm-otter-3f", last_activity: 300 }),
        entry("a", { cwd: "/home/u/forge", last_activity: 100 }),
      ],
      "updated",
    );
    expect(groups).toHaveLength(1);
    expect(groups[0].root).toBe("/home/u/forge");
    expect(groups[0].sessionCount).toBe(2);
  });

  it("groups by server and project, and counts a shared checkout's diff once", () => {
    const groups = buildProjectGroups(
      [
        entry("a", { diff_added: 10, diff_removed: 2 }),
        entry("b", { diff_added: 10, diff_removed: 2 }),
        entry("c", { cwd: "/home/u/forge/.forge/worktrees/abcd1234", diff_added: 5, diff_removed: 0 }),
        entry("other", { cwd: "/home/u/site", diff_added: 1 }),
      ],
      "updated",
    );
    const forge = groups.find((g) => g.name === "forge");
    expect(forge?.added).toBe(15);
    expect(forge?.removed).toBe(2);
    expect(groups.find((g) => g.name === "site")?.added).toBe(1);
  });
});

describe("filterByWorkspace", () => {
  it("passes everything without a workspace and narrows to its projects with one", () => {
    const rows = [entry("a"), entry("b", { cwd: "/home/u/site" })];
    expect(filterByWorkspace(rows, null)).toHaveLength(2);
    expect(filterByWorkspace(rows, new Set(["site"])).map((e) => e.row.id)).toEqual(["b"]);
  });
});

describe("labels", () => {
  it("shows a diff only when something changed", () => {
    expect(diffLabel(0, 0)).toBeNull();
    expect(diffLabel(null, undefined)).toBeNull();
    expect(diffLabel(12, 0)).toBe("+12");
    expect(diffLabel(12, 3)).toBe("+12 -3");
  });

  it("flags a failed turn only on a quiet session", () => {
    expect(failedTurn({ busy: false, waiting: false, last_turn_outcome: "failed" })).toBe(true);
    expect(failedTurn({ busy: true, waiting: false, last_turn_outcome: "failed" })).toBe(false);
    expect(failedTurn({ busy: false, waiting: false, last_turn_outcome: "success" })).toBe(false);
  });
});

describe("projectAccent", () => {
  it("is stable per name and lighter on a dark theme", () => {
    expect(projectAccent("forge", "dark")).toBe(projectAccent("forge", "dark"));
    expect(projectAccent("forge", "dark")).not.toBe(projectAccent("site", "dark"));
    expect(projectAccent("forge", "dark")).toContain("62%");
    expect(projectAccent("forge", "light")).toContain("42%");
  });
});

describe("starred and done", () => {
  it("floats starred sessions under the ones that need a decision", () => {
    const rows = [entry("recent", { last_activity: 900 }), entry("starred", { starred: true, last_activity: 1 }), entry("wait", { waiting: true })];
    expect(sortSessions(rows, "updated").map((e) => e.row.id)).toEqual(["wait", "starred", "recent"]);
  });

  it("splits done sessions out of the active list", () => {
    const { active, done } = splitDone([entry("a"), entry("b", { done_at: 5 })]);
    expect(active.map((e) => e.row.id)).toEqual(["a"]);
    expect(done.map((e) => e.row.id)).toEqual(["b"]);
  });
});
