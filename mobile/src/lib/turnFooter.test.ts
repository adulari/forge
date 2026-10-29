import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { buildTranscript } from "./toolRows";
import { foldToolGroups } from "./toolGroups";
import { formatTokens, formatWorked, turnFooterText, withTurnFooters } from "./turnFooter";

let seq = 0;
function row(over: Partial<HistoryRow> & Pick<HistoryRow, "role" | "created_at">): HistoryRow {
  seq += 1;
  return { seq, content: "x", model: null, visibility: "llm", kind: over.role === "tool" ? "tool" : over.role, ...over } as HistoryRow;
}

function fold(rowsChronological: HistoryRow[], busy = false) {
  const newestFirst = [...rowsChronological].reverse();
  return withTurnFooters(foldToolGroups(buildTranscript(newestFirst), busy), busy);
}

describe("turn footer text", () => {
  it("joins time, tokens and cost", () => {
    expect(turnFooterText({ seconds: 23, tokens: 4100, costUsd: 0.04 })).toBe("Worked for 23s · 4.1k tokens · $0.04");
  });

  it("leaves out what is unknown or zero", () => {
    expect(turnFooterText({ seconds: 0, tokens: null, costUsd: null })).toBe("");
    expect(turnFooterText({ seconds: 70, tokens: 900, costUsd: 0 })).toBe("Worked for 1m 10s · 900 tokens");
  });

  it("formats durations and counts", () => {
    expect(formatWorked(3725)).toBe("1h 02m");
    expect(formatTokens(1_250_000)).toBe("1.3M");
    expect(formatTokens(2000)).toBe("2k");
  });
});

describe("withTurnFooters", () => {
  it("closes a finished turn with its elapsed time, summed tokens and cost", () => {
    const entries = fold([
      row({ role: "user", created_at: 100 }),
      row({ role: "assistant", created_at: 110, content: "", input_tokens: 1000, output_tokens: 50, cost_usd: 0.01 }),
      row({ role: "tool", created_at: 110, tool: "shell", tool_phase: "call", input_tokens: 1000, output_tokens: 50, cost_usd: 0.01 }),
      row({ role: "tool", created_at: 118, tool: "shell", tool_phase: "result", content: "shell: exit 0" }),
      row({ role: "assistant", created_at: 123, content: "done", input_tokens: 3000, output_tokens: 50, cost_usd: 0.03 }),
    ]);
    const footers = entries.filter((e) => e.kind === "turnFooter");
    expect(footers).toHaveLength(1);
    expect(footers[0]).toMatchObject({ text: "Worked for 23s · 4.1k tokens · $0.04" });
    expect(entries[0].kind).toBe("turnFooter");
  });

  it("holds the newest turn's footer back while the session is busy", () => {
    const rows = [
      row({ role: "user", created_at: 10 }),
      row({ role: "assistant", created_at: 20, input_tokens: 10, output_tokens: 5 }),
    ];
    expect(fold(rows, true).some((e) => e.kind === "turnFooter")).toBe(false);
    expect(fold(rows, false).some((e) => e.kind === "turnFooter")).toBe(true);
  });

  it("puts each footer between its turn and the next person message", () => {
    const entries = fold([
      row({ role: "user", created_at: 0, content: "one" }),
      row({ role: "assistant", created_at: 5, content: "a1", input_tokens: 5, output_tokens: 5 }),
      row({ role: "user", created_at: 50, content: "two" }),
      row({ role: "assistant", created_at: 52, content: "a2", input_tokens: 5, output_tokens: 5 }),
    ]);
    const shape = entries.map((e) => (e.kind === "message" ? e.row.content : e.kind));
    expect(shape).toEqual(["turnFooter", "a2", "two", "turnFooter", "a1", "one"]);
  });

  it("skips a turn whose opening message is not loaded", () => {
    const entries = fold([row({ role: "assistant", created_at: 20, input_tokens: 10, output_tokens: 5 })]);
    expect(entries.some((e) => e.kind === "turnFooter")).toBe(false);
  });

  it("skips a turn with nothing measured to say", () => {
    const entries = fold([row({ role: "user", created_at: 5 }), row({ role: "assistant", created_at: 5 })]);
    expect(entries.some((e) => e.kind === "turnFooter")).toBe(false);
  });
});
