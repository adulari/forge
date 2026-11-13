import { describe, expect, it } from "vitest";

import type { GitDiffFile } from "../../lib/api";
import { acceptMessage, planAcceptRoutine, routineStageRequests, undoStageRequests } from "./acceptRoutine";
import { buildQueue, riskLine, riskRank, riskTotals, type ChangedFile, type QueueAgent } from "./changeQueueModel";

const file = (path: string, extra: Partial<ChangedFile> = {}): ChangedFile => ({ path, status: "M", adds: 1, dels: 0, hash: `h-${path}`, ...extra });
const agent = (sessionId: string, files: ChangedFile[]): QueueAgent => ({ sessionId, name: sessionId, waiting: false, busy: false, lastActivity: 0, files });

const routine = (path: string, hunks = 1) => file(path, { risk: "routine", riskReason: "lockfile churn", lookHunks: 0, routineHunks: hunks });
const look = (path: string, lookHunks = 1, routineHunks = 0) => file(path, { risk: "look", riskReason: "deletes a test", lookHunks, routineHunks });

describe("queue risk ranking", () => {
  it("sorts files that need a look ahead of routine ones inside a tier", () => {
    const groups = buildQueue({ agents: [agent("a", [routine("Cargo.lock"), file("big.ts", { adds: 90 }), look("a_test.rs")])], marks: {}, scope: null });
    expect(groups[0].rows.map((row) => row.path)).toEqual(["big.ts", "a_test.rs", "Cargo.lock"]);
    expect(riskRank(routine("x"))).toBe(1);
    expect(riskRank(file("x"))).toBe(0);
  });

  it("counts hunks over unreviewed files and words them as the queue header", () => {
    const rows = buildQueue({ agents: [agent("a", [routine("Cargo.lock", 3), look("t.rs", 2, 1), routine("done.lock", 5)])], marks: { "a\0done.lock": "h-done.lock" }, scope: null })[0].rows;
    const totals = riskTotals(rows);
    expect(totals).toMatchObject({ ranked: true, lookHunks: 2, routineHunks: 4, lookFiles: 1, routineFiles: 1 });
    expect(riskLine(totals)).toBe("2 look here · 4 routine");
    expect(riskLine({ lookHunks: 0, routineHunks: 3 })).toBe("3 routine");
  });

  it("shows no risk UI when the daemon sent no verdicts", () => {
    const rows = buildQueue({ agents: [agent("a", [file("a.ts")])], marks: {}, scope: null })[0].rows;
    expect(riskTotals(rows).ranked).toBe(false);
  });
});

describe("planAcceptRoutine", () => {
  const rows = buildQueue({
    agents: [agent("tree", [routine("Cargo.lock"), look("t.rs", 1, 2), look("x.rs")]), agent("wt", [routine("pnpm-lock.yaml"), look("m.rs", 1, 1)])],
    marks: {},
    scope: null,
  }).flatMap((group) => group.rows);

  it("stages shared-tree files with routine hunks and reviews fully routine worktree files", () => {
    const plan = planAcceptRoutine(rows, new Set(["wt"]), false);
    expect(plan.stage.map((row) => row.path).sort()).toEqual(["Cargo.lock", "t.rs"]);
    expect(plan.review.map((row) => row.path)).toEqual(["pnpm-lock.yaml"]);
  });

  it("does not stage over a relayed connection", () => {
    const plan = planAcceptRoutine(rows, new Set(["wt"]), true);
    expect(plan.stage).toEqual([]);
    expect(plan.review).toHaveLength(1);
  });
});

describe("routineStageRequests", () => {
  const diff: GitDiffFile = {
    path: "src/a.rs",
    kind: "modified",
    orig_path: null,
    binary: false,
    adds: 3,
    dels: 1,
    skipped_lines: 0,
    hunks: [
      { header: "@@ -1,2 +1,2 @@", lines: ["-use b;", "+use a;"], risk: "routine", reason: "import reorder" },
      { header: "@@ -10,2 +10,2 @@", lines: ["-x()", "+y()"], risk: "look", reason: "removes 1 line" },
      { header: "@@ -20,1 +20,2 @@", lines: [" a", "+ "], risk: "routine", reason: "whitespace only" },
    ],
  };

  it("builds a stage request per routine hunk, last first", () => {
    const requests = routineStageRequests("s1", diff);
    expect(requests.map((r) => r.hunk_header)).toEqual(["@@ -20,1 +20,2 @@", "@@ -1,2 +1,2 @@"]);
    expect(requests.every((r) => r.action === "stage" && r.session === "s1" && r.path === "src/a.rs")).toBe(true);
    expect(requests[1].patch).toBe("--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n-use b;\n+use a;\n");
  });

  it("takes nothing from a diff a patch cannot rebuild", () => {
    expect(routineStageRequests("s", { ...diff, skipped_lines: 4 })).toEqual([]);
    expect(routineStageRequests("s", { ...diff, binary: true })).toEqual([]);
    expect(routineStageRequests("s", { ...diff, kind: "renamed" })).toEqual([]);
  });

  it("undoes by unstaging the same hunks in reverse order", () => {
    const requests = routineStageRequests("s1", diff);
    const undo = undoStageRequests(requests);
    expect(undo.map((r) => r.hunk_header)).toEqual(["@@ -1,2 +1,2 @@", "@@ -20,1 +20,2 @@"]);
    expect(undo.every((r) => r.action === "unstage")).toBe(true);
  });
});

describe("acceptMessage", () => {
  it("reads as one sentence", () => {
    expect(acceptMessage({ hunks: 31, files: 12, failed: 0 }, 0)).toBe("Staged 31 routine hunks in 12 files");
    expect(acceptMessage({ hunks: 1, files: 1, failed: 2 }, 3)).toBe("Staged 1 routine hunk in 1 file, marked 3 routine files reviewed. 2 could not be applied (the file changed).");
    expect(acceptMessage({ hunks: 0, files: 0, failed: 0 }, 0)).toBe("Nothing routine to accept");
  });
});
