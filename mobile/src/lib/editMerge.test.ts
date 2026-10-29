import { describe, expect, it } from "vitest";

import { mergeEdits } from "./editMerge";
import type { CallStatus, ToolGroupCall } from "./toolGroups";

function call(key: string, tool: string, args: object, over: Partial<{ status: CallStatus; adds: number; dels: number; result: string | null }> = {}): ToolGroupCall {
  return {
    key,
    status: over.status ?? "ok",
    invocation: { seq: 1, tool, args: JSON.stringify(args), result: over.result === undefined ? "ok" : over.result, adds: over.adds, dels: over.dels },
  };
}

describe("mergeEdits", () => {
  it("folds repeated edits to one file into one row with a combined counter", () => {
    const rows = mergeEdits([
      call("a", "edit_file", { path: "src/a.ts" }, { adds: 3, dels: 1 }),
      call("b", "read_file", { path: "src/b.ts" }),
      call("c", "edit_file", { path: "./src/a.ts" }, { adds: 2, dels: 0 }),
      call("d", "write_file", { path: "src/a.ts" }, { adds: 4, dels: 0 }),
    ]);
    expect(rows.map((r) => r.kind)).toEqual(["edits", "call"]);
    expect(rows[0]).toMatchObject({ kind: "edits", path: "src/a.ts", adds: 9, dels: 1 });
    expect(rows[0].kind === "edits" && rows[0].calls).toHaveLength(3);
  });

  it("leaves a counter out when any edit's lines are unknown", () => {
    const rows = mergeEdits([call("a", "edit_file", { path: "x" }, { adds: 3, dels: 1 }), call("b", "edit_file", { path: "x" })]);
    expect(rows[0]).toMatchObject({ kind: "edits", adds: null, dels: null });
  });

  it("keeps a single edit, a failed edit and a running edit as their own rows", () => {
    const rows = mergeEdits([
      call("a", "edit_file", { path: "x" }),
      call("b", "edit_file", { path: "y" }),
      call("c", "edit_file", { path: "y" }, { status: "failed", result: "error: no match" }),
      call("d", "edit_file", { path: "y" }, { status: "running", result: null }),
    ]);
    expect(rows.every((r) => r.kind === "call")).toBe(true);
  });
});
