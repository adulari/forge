import { describe, expect, it } from "vitest";

import type { GitDiffFile, GitStatusResponse, SessionRow } from "../../lib/api";
import { defaultProject, filesFromGitStatus, filesFromSessionDiff, planChangeQueries, projectSessions } from "./changesAdapter";

const row = (id: string, extra: Partial<SessionRow> = {}): SessionRow =>
  ({
    id,
    title: id,
    cwd: "/work/forge",
    worktree: null,
    busy: false,
    waiting: false,
    last_activity: 0,
    ...extra,
  }) as SessionRow;

const gitFile = (path: string, line: string): GitDiffFile => ({
  path,
  kind: "modified",
  orig_path: null,
  binary: false,
  adds: 1,
  dels: 0,
  skipped_lines: 0,
  hunks: [{ header: "@@ -1 +1 @@", lines: [line] }],
});

describe("changes adapter", () => {
  it("hashes a worktree diff by its content", () => {
    const [a] = filesFromSessionDiff([gitFile("a.ts", "+one")]);
    const [b] = filesFromSessionDiff([gitFile("a.ts", "+two")]);
    const [same] = filesFromSessionDiff([gitFile("a.ts", "+one")]);
    expect(a.hash).not.toBe(b.hash);
    expect(a.hash).toBe(same.hash);
  });

  it("lists a git-status path once even when it is staged and unstaged, and flags conflicts", () => {
    const f = (path: string, status = "M") => ({ path, status, orig_path: null, adds: 1, dels: 1, binary: false });
    const status = {
      staged: [f("a.ts")],
      unstaged: [f("a.ts"), f("b.ts")],
      untracked: [f("c.ts", "?")],
      conflicted: [f("d.ts", "U")],
    } as unknown as GitStatusResponse;
    const files = filesFromGitStatus(status);
    expect(files.map((x) => x.path).sort()).toEqual(["a.ts", "b.ts", "c.ts", "d.ts"]);
    expect(files.find((x) => x.path === "d.ts")?.conflicted).toBe(true);
    expect(files.find((x) => x.path === "a.ts")?.conflicted).toBe(false);
  });

  it("orders a project's sessions needs-you first", () => {
    const rows = [
      row("idle", { last_activity: 9 }),
      row("busy", { busy: true, last_activity: 1 }),
      row("wait", { waiting: true, last_activity: 0 }),
      row("elsewhere", { cwd: "/work/other", waiting: true }),
    ];
    expect(projectSessions(rows, "forge").map((r) => r.id)).toEqual(["wait", "busy", "idle"]);
    expect(projectSessions(rows, null)).toEqual([]);
  });

  it("opens on the routed session's project, then a waiting one, then the latest", () => {
    const rows = [row("a", { cwd: "/w/alpha", last_activity: 5 }), row("b", { cwd: "/w/beta", waiting: true, last_activity: 1 })];
    expect(defaultProject(rows, "a")).toBe("alpha");
    expect(defaultProject(rows, null)).toBe("beta");
    expect(defaultProject([row("a", { cwd: "/w/alpha" })], "missing")).toBe("alpha");
    expect(defaultProject([], null)).toBeNull();
  });

  it("asks a shared checkout once and points the other sessions at its holder", () => {
    const plan = planChangeQueries([
      row("old", { last_activity: 1 }),
      row("new", { last_activity: 5 }),
      row("tree", { worktree: "/wt/x", cwd: "/wt/x", last_activity: 2 }),
    ]);
    expect(plan.map((p) => [p.row.id, p.sharedWith])).toEqual([
      ["old", "new"],
      ["new", null],
      ["tree", null],
    ]);
  });
});
