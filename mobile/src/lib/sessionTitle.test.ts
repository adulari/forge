import { describe, expect, it } from "vitest";

import type { HistoryRow } from "./api";
import { sessionDisplayTitle } from "./sessionTitle";

const row = (seq: number, role: HistoryRow["role"], content: string, kind?: HistoryRow["kind"]): HistoryRow => ({
  seq, role, content, model: null, created_at: 0, visibility: "llm", kind,
});

describe("sessionDisplayTitle", () => {
  it("prefers the daemon's title", () => {
    expect(sessionDisplayTitle("Fix the parser", [], "db89c45e-1")).toBe("Fix the parser");
  });

  it("names an untitled session by its first prompt, skipping nudges and attachments", () => {
    const rows = [
      row(4, "assistant", "ok"),
      row(3, "user", "second ask"),
      row(2, "user", "keep going", "nudge"),
      row(1, "user", "@/tmp/.forge/uploads/s/1-a.png\ntrack the solver   speed"),
    ];
    expect(sessionDisplayTitle("", rows, "db89c45e-1")).toBe("track the solver speed");
  });

  it("falls back to the short id and caps long prompts", () => {
    expect(sessionDisplayTitle(null, [], "db89c45e-1234")).toBe("session db89c45e");
    expect(sessionDisplayTitle("", [row(1, "user", "x".repeat(200))], "s").length).toBe(72);
  });
});
