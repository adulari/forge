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
    expect(resultFailed("permission denied by policy")).toBe(true);
  });

  it("reads the shell exit code, not words in the output", () => {
    expect(resultFailed("shell: exit 0 in 12ms\n\nerror: none, test result: 1 failed")).toBe(false);
    expect(resultFailed("shell: exit 101 in 3ms\n\nok")).toBe(true);
    expect(resultFailed("shell: exit 0 in 5ms (2/3 ok)")).toBe(false);
    expect(resultFailed("shell: timed out after 30s (killed) in 30000ms")).toBe(true);
  });

  it("does not flag a successful result that mentions errors", () => {
    expect(resultFailed("shell: exit 0 in 241ms\n\n5\n\n[run_code: 3 tool calls, 3 ok, 0 failed, 97.4s]")).toBe(false);
    expect(resultFailed("fn handle_error() -> Result<(), Error> {}")).toBe(false);
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

  it("pairs parallel calls to one tool with their results in call order, one row per call", () => {
    // newest-first: three run_code calls were served first, then their three results
    const rows = [
      row({ seq: 7, role: "tool", kind: "tool", tool: "run_code", tool_phase: "result", content: "error: third" }),
      row({ seq: 6, role: "tool", kind: "tool", tool: "run_code", tool_phase: "result", content: "error: second" }),
      row({ seq: 5, role: "tool", kind: "tool", tool: "run_code", tool_phase: "result", content: "ok first" }),
      row({ seq: 4, role: "tool", kind: "tool", tool: "run_code", tool_phase: "call", content: '{"code":"c"}' }),
      row({ seq: 3, role: "tool", kind: "tool", tool: "run_code", tool_phase: "call", content: '{"code":"b"}' }),
      row({ seq: 2, role: "tool", kind: "tool", tool: "run_code", tool_phase: "call", content: '{"code":"a"}' }),
      row({ seq: 1, role: "user", kind: "user", content: "go" }),
    ];
    const tools = buildTranscript(rows).filter((e) => e.kind === "tool");
    expect(tools).toHaveLength(3);
    expect(tools.map((e) => (e.kind === "tool" ? [e.invocation.args, e.invocation.result] : null))).toEqual([
      ['{"code":"c"}', "error: third"],
      ['{"code":"b"}', "error: second"],
      ['{"code":"a"}', "ok first"],
    ]);
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

  it("drops a whitespace-only assistant turn instead of rendering an empty bubble", () => {
    // Reproduces the reported bug: the daemon persisted "\n\n\n" for the assistant row of a turn
    // whose real prose lived only on tool-call rows this page never fetched — rendering that
    // blank content produced a "Forge" bubble with nothing under it, which read as the reply
    // having vanished.
    const rows = [
      row({ seq: 3, role: "assistant", kind: "assistant", content: "\n\n\n" }),
      row({ seq: 2, role: "assistant", kind: "assistant", content: "Done." }),
      row({ seq: 1, role: "user", kind: "user", content: "reply with pong" }),
    ];
    const out = buildTranscript(rows);
    expect(out.map((e) => e.kind)).toEqual(["message", "message"]);
    expect(out.map((e) => (e.kind === "message" ? e.row.seq : null))).toEqual([2, 1]);
  });

  it("never drops a whitespace-prefixed kind:\"system\" completion note, even though it carries role: assistant", () => {
    const rows = [
      row({ seq: 1, role: "assistant", kind: "system", visibility: "ui", content: "\n\nAll tasks complete." }),
    ];
    const out = buildTranscript(rows);
    expect(out).toHaveLength(1);
    expect(out[0].kind).toBe("message");
  });

  it("never drops a genuinely blank user turn (the filter is assistant-only)", () => {
    const rows = [row({ seq: 1, role: "user", kind: "user", content: "" })];
    expect(buildTranscript(rows)).toHaveLength(1);
  });
});
