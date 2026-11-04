import { describe, expect, it } from "vitest";

import type { ProjectChangesResponse, SessionRow } from "../../lib/api";
import { buildQueue, isReviewed, markReviewed, pruneReviewed } from "./changeQueueModel";
import { changesCwd, routeMissing, sessionChangesFromRoute } from "./changesFromRoute";

const row = (id: string, extra: Partial<SessionRow> = {}) => ({ id, cwd: "/r", worktree: null, last_activity: 1, ...extra }) as unknown as SessionRow;
const file = (path: string, hash: string) => ({ path, kind: "modified" as const, adds: 1, dels: 0, content_hash: hash });
const group = (id: string, files: ReturnType<typeof file>[]) => ({ session_id: id, title: id, branch: "b", worktree: null, waiting: false, busy: false, files, truncated: 0 });

describe("sessionChangesFromRoute", () => {
  it("maps files with the daemon's content hash", () => {
    const res: ProjectChangesResponse = { cwd: "/r", sessions: [group("a", [file("x.rs", "h1")])] };
    const map = sessionChangesFromRoute(res, [row("a"), row("b")]);
    expect(map.get("a")?.files[0]).toMatchObject({ path: "x.rs", hash: "h1", status: "M" });
    expect(map.get("b")?.files).toEqual([]);
  });
  it("hands unattributed files to the newest shared-tree session, without duplicates", () => {
    const res: ProjectChangesResponse = {
      cwd: "/r",
      sessions: [group("a", [file("x.rs", "h1")]), group("unattributed", [file("x.rs", "h9"), file("y.rs", "h2")])],
    };
    const map = sessionChangesFromRoute(res, [row("a", { last_activity: 5 }), row("w", { worktree: "/w", last_activity: 9 })]);
    expect(map.get("a")?.files.map((f) => f.path)).toEqual(["x.rs", "y.rs"]);
    expect(map.get("w")?.files).toEqual([]);
  });
  it("drops groups for sessions that are not on the project", () => {
    const res: ProjectChangesResponse = { cwd: "/r", sessions: [group("zzz", [file("x.rs", "h")])] };
    expect([...sessionChangesFromRoute(res, [row("a")]).values()].flatMap((c) => c.files)).toEqual([]);
  });
});

describe("reviewed marks follow the content hash", () => {
  it("clears when the hash changes even though the +/- counts are equal", () => {
    const before = sessionChangesFromRoute({ cwd: "/r", sessions: [group("a", [file("x.rs", "h1")])] }, [row("a")]);
    const marks = markReviewed({}, "a", "x.rs", before.get("a")!.files[0].hash);
    const after = sessionChangesFromRoute({ cwd: "/r", sessions: [group("a", [file("x.rs", "h2")])] }, [row("a")]);
    const f = after.get("a")!.files[0];
    expect(isReviewed(marks, "a", "x.rs", f.hash)).toBe(false);
    const queue = buildQueue({ agents: [{ sessionId: "a", name: "a", waiting: false, busy: false, lastActivity: 1, files: after.get("a")!.files }], marks, scope: null });
    expect(queue[0].rows[0].tier).toBe("unreviewed");
    expect(Object.keys(pruneReviewed(marks, new Map([["a", new Map([["x.rs", "h2"]])]])))).toEqual([]);
  });
  it("keeps the mark while the hash holds", () => {
    const marks = markReviewed({}, "a", "x.rs", "h1");
    const queue = buildQueue({ agents: [{ sessionId: "a", name: "a", waiting: false, busy: false, lastActivity: 1, files: [{ path: "x.rs", status: "M", adds: 1, dels: 0, hash: "h1" }] }], marks, scope: null });
    expect(queue[0].rows[0].tier).toBe("reviewed");
  });
});

describe("routeMissing / changesCwd", () => {
  it("falls back only on 404 and 405", () => {
    expect(routeMissing(Object.assign(new Error("x"), { status: 404 }))).toBe(true);
    expect(routeMissing(Object.assign(new Error("x"), { status: 405 }))).toBe(true);
    expect(routeMissing(Object.assign(new Error("x"), { status: 500 }))).toBe(false);
    expect(routeMissing(new Error("offline"))).toBe(false);
  });
  it("prefers the shared checkout's directory", () => {
    expect(changesCwd([row("w", { cwd: "/w", worktree: "/w" }), row("a", { cwd: "/r" })])).toBe("/r");
    expect(changesCwd([])).toBeNull();
  });
});
