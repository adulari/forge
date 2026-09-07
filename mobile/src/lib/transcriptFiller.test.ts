import { describe, expect, it } from "vitest";

import { buildTranscript } from "./toolRows";
import { fillerHistoryRows } from "./transcriptFiller";
import type { TranscriptRow } from "./ws";

const rows: TranscriptRow[] = [
  { kind: "user", text: "fix the bug" },
  { kind: "assistant", text: "Looking." },
  { kind: "tool", text: '{"command":"ls"}', tool: "shell", meta: null },
  { kind: "tool", text: "exit 0", tool: "shell", meta: "ok" },
  { kind: "assistant", text: "Done." },
  { kind: "user", text: "and now?" },
  { kind: "assistant", text: "Working on it" },
];

describe("fillerHistoryRows", () => {
  it("returns null for a pre-v9 host so the plain-line fallback stays", () => {
    expect(fillerHistoryRows(undefined, false)).toBeNull();
    expect(fillerHistoryRows([], false)).toBeNull();
  });

  it("maps provenance to the roles history uses, newest first, and pairs tool call/result", () => {
    const history = fillerHistoryRows(rows, false)!;
    expect(history.map((r) => r.role)).toEqual([
      "assistant", "user", "assistant", "tool", "tool", "assistant", "user",
    ]);
    const entries = buildTranscript(history);
    const tools = entries.filter((e) => e.kind === "tool");
    expect(tools).toHaveLength(1);
    expect(tools[0].kind === "tool" && tools[0].invocation).toMatchObject({
      tool: "shell",
      args: '{"command":"ls"}',
      result: "exit 0",
    });
  });

  it("drops the in-flight turn from the last user row onward when asked", () => {
    const history = fillerHistoryRows(rows, true)!;
    expect(history.map((r) => r.content)).toEqual(["Done.", "exit 0", '{"command":"ls"}', "Looking.", "fix the bug"]);
  });

  it("uses positive, unique synthetic seqs", () => {
    const seqs = fillerHistoryRows(rows, false)!.map((r) => r.seq);
    expect(new Set(seqs).size).toBe(seqs.length);
    expect(Math.min(...seqs)).toBeGreaterThan(0);
  });
});
