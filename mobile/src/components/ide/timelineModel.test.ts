import { describe, expect, it } from "vitest";

import type { HistoryRow } from "../../lib/api";
import { buildTimeline, findTurnItem, resolveChangedPath, toolKind, type TimelineItem } from "./timelineModel";

const row = (seq: number, extra: Partial<HistoryRow>): HistoryRow =>
  ({ seq, role: "assistant", content: "", model: null, created_at: seq, visibility: "llm", ...extra }) as HistoryRow;

describe("toolKind", () => {
  it("sorts tools into what the timeline icons mean", () => {
    expect(toolKind("write_file")).toBe("edit");
    expect(toolKind("edit_file")).toBe("edit");
    expect(toolKind("shell")).toBe("command");
    expect(toolKind("read_file")).toBe("read");
    expect(toolKind("grep")).toBe("read");
    expect(toolKind("web_fetch")).toBe("tool");
    expect(toolKind(null)).toBe("tool");
  });
});

describe("buildTimeline", () => {
  it("pairs calls with results, labels them by target and lists newest first", () => {
    const rows: HistoryRow[] = [
      row(5, { kind: "tool", role: "tool", tool: "shell", tool_phase: "result", content: "shell: exit 1\nfail" }),
      row(4, { kind: "tool", role: "tool", tool: "shell", tool_phase: "call", content: '{"command":"cargo test"}' }),
      row(3, { kind: "tool", role: "tool", tool: "write_file", tool_phase: "result", content: "ok" }),
      row(2, { kind: "tool", role: "tool", tool: "write_file", tool_phase: "call", content: '{"path":"/repo/src/a.ts","content":"x"}' }),
      row(1, { role: "user", content: "fix the bug\nplease" }),
    ];
    const items = buildTimeline(rows);
    expect(items.map((i) => [i.kind, i.label])).toEqual([
      ["command", "cargo test"],
      ["edit", "/repo/src/a.ts"],
      ["prompt", "fix the bug"],
    ]);
    expect(items[0].failed).toBe(true);
    expect(items[1].path).toBe("/repo/src/a.ts");
  });

  it("caps the list", () => {
    const rows = Array.from({ length: 100 }, (_, i) => row(100 - i, { role: "user", content: `p${i}` }));
    expect(buildTimeline(rows, 10)).toHaveLength(10);
  });
});

describe("resolveChangedPath", () => {
  it("maps an absolute written path onto the changed file by whole segments", () => {
    expect(resolveChangedPath("/repo/src/a.ts", ["src/a.ts", "a.ts"])).toBe("src/a.ts");
    expect(resolveChangedPath("src/a.ts", ["src/a.ts"])).toBe("src/a.ts");
    expect(resolveChangedPath("/repo/xsrc/a.ts", ["src/a.ts"])).toBeNull();
    expect(resolveChangedPath("/repo/other.ts", ["src/a.ts"])).toBeNull();
  });
});

describe("findTurnItem", () => {
  const item = (seq: number, kind: TimelineItem["kind"], path: string | null = null): TimelineItem => ({ key: `k${seq}`, kind, label: "", tool: null, failed: false, adds: 0, dels: 0, path, seq });
  const items = [item(12, "edit", "/r/src/b.rs"), item(9, "edit", "/r/src/a.rs"), item(8, "read"), item(5, "edit", "/r/src/a.rs")];
  it("picks the earliest edit on the path at or after the turn", () => {
    expect(findTurnItem(items, 4, "src/a.rs")).toBe(3);
    expect(findTurnItem(items, 6, "src/a.rs")).toBe(1);
  });
  it("falls back to the nearest row after the turn, and -1 when none", () => {
    expect(findTurnItem(items, 7, "src/zzz.rs")).toBe(2);
    expect(findTurnItem(items, 99, "src/a.rs")).toBe(-1);
  });
});
