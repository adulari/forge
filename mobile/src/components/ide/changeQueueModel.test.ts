import { describe, expect, it } from "vitest";

import {
  buildQueue,
  fingerprint,
  flattenQueue,
  isReviewed,
  markReviewed,
  nextToReview,
  pruneReviewed,
  reviewKey,
  stepQueue,
  unmarkReviewed,
  type ChangedFile,
  type QueueAgent,
} from "./changeQueueModel";

const file = (path: string, extra: Partial<ChangedFile> = {}): ChangedFile => ({
  path,
  status: "M",
  adds: 1,
  dels: 0,
  hash: `h-${path}`,
  ...extra,
});

const agent = (sessionId: string, files: ChangedFile[], extra: Partial<QueueAgent> = {}): QueueAgent => ({
  sessionId,
  name: sessionId,
  waiting: false,
  busy: false,
  lastActivity: 0,
  files,
  ...extra,
});

describe("buildQueue attention ordering", () => {
  it("orders files waiting, conflict, failing, unreviewed, reviewed across and within agents", () => {
    const marks = markReviewed({}, "a", "done.ts", "h-done.ts");
    const groups = buildQueue({
      agents: [
        agent("a", [file("done.ts"), file("plain.ts"), file("fail.ts", { failing: true }), file("clash.ts", { conflicted: true })]),
        agent("b", [file("needs.ts")], { waiting: true }),
      ],
      marks,
      scope: null,
    });
    expect(groups.map((g) => g.sessionId)).toEqual(["b", "a"]);
    expect(groups[0].tier).toBe("waiting");
    expect(groups[1].rows.map((r) => [r.path, r.tier])).toEqual([
      ["clash.ts", "conflict"],
      ["fail.ts", "failing"],
      ["plain.ts", "unreviewed"],
      ["done.ts", "reviewed"],
    ]);
  });

  it("puts a waiting agent's reviewed file ahead of everything: the agent is the blocker", () => {
    const marks = markReviewed({}, "w", "x.ts", "h-x.ts");
    const [group] = buildQueue({ agents: [agent("w", [file("x.ts")], { waiting: true })], marks, scope: null });
    expect(group.rows[0].tier).toBe("waiting");
  });

  it("breaks ties inside a tier by churn, then path", () => {
    const [group] = buildQueue({
      agents: [agent("a", [file("b.ts", { adds: 2 }), file("a.ts", { adds: 2 }), file("big.ts", { adds: 30, dels: 4 })])],
      marks: {},
      scope: null,
    });
    expect(group.rows.map((r) => r.path)).toEqual(["big.ts", "a.ts", "b.ts"]);
  });

  it("breaks group ties by most recent activity and drops groups without files", () => {
    const groups = buildQueue({
      agents: [
        agent("old", [file("a.ts")], { lastActivity: 1 }),
        agent("new", [file("b.ts")], { lastActivity: 9 }),
        agent("empty", []),
      ],
      marks: {},
      scope: null,
    });
    expect(groups.map((g) => g.sessionId)).toEqual(["new", "old"]);
  });

  it("scopes to one agent", () => {
    const groups = buildQueue({
      agents: [agent("a", [file("a.ts")]), agent("b", [file("b.ts")])],
      marks: {},
      scope: "b",
    });
    expect(groups.map((g) => g.sessionId)).toEqual(["b"]);
  });

  it("writes a plain one-line summary", () => {
    const marks = markReviewed({}, "a", "a.ts", "h-a.ts");
    const [group] = buildQueue({
      agents: [agent("a", [file("a.ts", { adds: 4, dels: 1 }), file("b.ts", { adds: 2, dels: 2 })])],
      marks,
      scope: null,
    });
    expect(group.summary).toBe("1 to review · 2 files · +6 −3");
    const [done] = buildQueue({ agents: [agent("a", [file("a.ts")])], marks, scope: null });
    expect(done.summary).toContain("All reviewed");
  });
});

describe("reviewed marks", () => {
  it("honours a mark only while the content hash is unchanged", () => {
    const marks = markReviewed({}, "s", "f.ts", "v1");
    expect(isReviewed(marks, "s", "f.ts", "v1")).toBe(true);
    expect(isReviewed(marks, "s", "f.ts", "v2")).toBe(false);
    expect(isReviewed(marks, "other", "f.ts", "v1")).toBe(false);
  });

  it("a file edited again drops back to unreviewed in the queue", () => {
    const marks = markReviewed({}, "s", "f.ts", "v1");
    const reviewed = buildQueue({ agents: [agent("s", [file("f.ts", { hash: "v1" })])], marks, scope: null });
    const edited = buildQueue({ agents: [agent("s", [file("f.ts", { hash: "v2" })])], marks, scope: null });
    expect(reviewed[0].rows[0].tier).toBe("reviewed");
    expect(edited[0].rows[0].tier).toBe("unreviewed");
  });

  it("prunes marks whose hash moved or whose file left, keeps sessions that were not loaded", () => {
    let marks = markReviewed({}, "s", "kept.ts", "a");
    marks = markReviewed(marks, "s", "moved.ts", "a");
    marks = markReviewed(marks, "s", "gone.ts", "a");
    marks = markReviewed(marks, "unloaded", "x.ts", "a");
    const live = new Map([["s", new Map([["kept.ts", "a"], ["moved.ts", "b"]])]]);
    const pruned = pruneReviewed(marks, live);
    expect(Object.keys(pruned).sort()).toEqual([reviewKey("s", "kept.ts"), reviewKey("unloaded", "x.ts")].sort());
  });

  it("returns the same object when nothing needs pruning or unmarking", () => {
    const marks = markReviewed({}, "s", "a.ts", "a");
    expect(pruneReviewed(marks, new Map([["s", new Map([["a.ts", "a"]])]]))).toBe(marks);
    expect(unmarkReviewed(marks, "s", "nope.ts")).toBe(marks);
    expect(unmarkReviewed(marks, "s", "a.ts")).toEqual({});
  });
});

describe("walking the queue", () => {
  const groups = buildQueue({
    agents: [agent("a", [file("a1.ts", { adds: 9 }), file("a2.ts", { adds: 5 })]), agent("b", [file("b1.ts")], { lastActivity: -1 })],
    marks: {},
    scope: null,
  });
  const rows = flattenQueue(groups);

  it("steps without wrapping and starts at an end when nothing is open", () => {
    expect(stepQueue(rows, null, 1)?.path).toBe("a1.ts");
    expect(stepQueue(rows, null, -1)?.path).toBe("b1.ts");
    expect(stepQueue(rows, rows[0].key, -1)?.path).toBe("a1.ts");
    expect(stepQueue(rows, rows[2].key, 1)?.path).toBe("b1.ts");
    expect(stepQueue([], null, 1)).toBeNull();
  });

  it("after review jumps to the next file that still needs a look", () => {
    const marks = markReviewed({}, "a", "a2.ts", "h-a2.ts");
    const next = flattenQueue(buildQueue({ agents: [agent("a", [file("a1.ts", { adds: 9 }), file("a2.ts", { adds: 5 })]), agent("b", [file("b1.ts")], { lastActivity: -1 })], marks, scope: null }));
    expect(nextToReview(next, next[0].key)?.path).toBe("b1.ts");
    expect(nextToReview(next.slice(0, 1), next[0].key)).toBeNull();
  });
});

describe("fingerprint", () => {
  it("is stable and sensitive to content and boundaries", () => {
    expect(fingerprint(["a", "b"])).toBe(fingerprint(["a", "b"]));
    expect(fingerprint(["ab"])).not.toBe(fingerprint(["a", "b"]));
  });
});
