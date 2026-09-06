import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import {
  buildTranscript,
  formatArgs,
  resultFailed,
  summarizeToolArgs,
  summarizeToolLine,
} from "./toolRows";

function row(partial: Partial<HistoryRow> & { seq: number }): HistoryRow {
  return {
    role: "assistant",
    content: "",
    model: null,
    created_at: 0,
    visibility: "llm",
    ...partial,
  } as HistoryRow;
}

describe("summarizeToolArgs", () => {
  it("pulls the primary target, never the raw blob", () => {
    expect(summarizeToolArgs('{"command":"scripts/ci/rust-checks.sh","cwd":"/x"}')).toBe(
      "scripts/ci/rust-checks.sh",
    );
    expect(summarizeToolArgs('{"path":"src/main.rs"}')).toBe("src/main.rs");
  });

  it("returns empty when no recognizable target (caller shows bare name)", () => {
    expect(summarizeToolArgs('{"unknown":"x"}')).toBe("");
    expect(summarizeToolArgs("not json")).toBe("");
    // store-capped, unparsable JSON must not leak
    expect(summarizeToolArgs('{"content":"aaaa…')).toBe("");
  });

  it("middle-truncates a long target", () => {
    const long = "a/very/long/path/that/keeps/going/and/going/forever.rs";
    const out = summarizeToolArgs(`{"path":"${long}"}`, 20);
    expect(out.length).toBeLessThanOrEqual(20);
    expect(out).toContain("…");
  });
});

describe("summarizeToolLine", () => {
  it("summarizes a transcript ↳ line and passes clean lines through", () => {
    expect(summarizeToolLine('↳ write_file {"path":"a.rs","content":"x"}')).toBe("write_file a.rs");
    expect(summarizeToolLine("you")).toBe("you");
    expect(summarizeToolLine('↳ shell {"nope":1}')).toBe("shell");
  });
});

describe("formatArgs", () => {
  it("pretty-prints parsable args and falls back to raw text otherwise", () => {
    expect(formatArgs('{"a":1}')).toBe('{\n  "a": 1\n}');
    expect(formatArgs('{"content":"aa…')).toBe('{"content":"aa…');
  });
});

describe("resultFailed", () => {
  it("flags error/failed text but not a passing result", () => {
    expect(resultFailed("Error: boom")).toBe(true);
    expect(resultFailed("42 passed; 0 failed")).toBe(false);
    expect(resultFailed("all good")).toBe(false);
    expect(resultFailed(null)).toBe(false);
  });
});

describe("buildTranscript", () => {
  it("pairs a call with its result into one tool entry", () => {
    // newest-first input: result (seq 3), call (seq 2), user (seq 1)
    const rows = [
      row({ seq: 3, role: "tool", kind: "tool", tool: "shell", tool_phase: "result", content: "hello\n" }),
      row({ seq: 2, role: "tool", kind: "tool", tool: "shell", tool_phase: "call", content: '{"command":"echo hello"}' }),
      row({ seq: 1, role: "user", kind: "user", content: "run it" }),
    ];
    const out = buildTranscript(rows);
    // newest-first output: [tool, user]
    expect(out.map((e) => e.kind)).toEqual(["tool", "message"]);
    const tool = out[0];
    if (tool.kind !== "tool") throw new Error("expected tool");
    expect(tool.invocation.tool).toBe("shell");
    expect(tool.invocation.args).toBe('{"command":"echo hello"}');
    expect(tool.invocation.result).toBe("hello\n");
  });

  it("keeps a call whose result never arrived (result null)", () => {
    const rows = [
      row({ seq: 2, role: "tool", kind: "tool", tool: "shell", tool_phase: "call", content: "{}" }),
      row({ seq: 1, role: "user", kind: "user", content: "go" }),
    ];
    const out = buildTranscript(rows);
    const tool = out[0];
    if (tool.kind !== "tool") throw new Error("expected tool");
    expect(tool.invocation.result).toBeNull();
  });

  it("renders a phase-less result row (pre-v10 daemon) as an args-less tool entry", () => {
    const rows = [
      row({ seq: 1, role: "tool", kind: "tool", tool: "read_file", content: "file body" }),
    ];
    const out = buildTranscript(rows);
    const tool = out[0];
    if (tool.kind !== "tool") throw new Error("expected tool");
    expect(tool.invocation.args).toBe("");
    expect(tool.invocation.result).toBe("file body");
  });

  it("leaves ordinary user/assistant turns as message entries in order", () => {
    const rows = [
      row({ seq: 2, role: "assistant", kind: "assistant", content: "hi" }),
      row({ seq: 1, role: "user", kind: "user", content: "hey" }),
    ];
    const out = buildTranscript(rows);
    expect(out.map((e) => e.kind)).toEqual(["message", "message"]);
  });
});
