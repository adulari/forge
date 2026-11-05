import { describe, expect, it } from "vitest";

import {
  claudePickerRows,
  continueToast,
  copyableCommand,
  linkedClaudeId,
  queueInitialCommand,
  shellQuote,
  takeInitialCommand,
} from "./claudeHandoff";
import type { ImportCandidate } from "./importSessions";

const row = (over: Partial<ImportCandidate>): ImportCandidate => ({
  agent: "claude",
  id: "a",
  title: "fix the flaky test",
  cwd: "/home/me/api",
  cwd_exists: true,
  modified: 100,
  bytes: 10,
  imported_session_id: null,
  ...over,
});

describe("claudePickerRows", () => {
  it("keeps only Claude Code conversations", () => {
    const rows = claudePickerRows([row({ id: "c1" }), row({ id: "x1", agent: "codex" })], "");
    expect(rows.map((r) => r.id)).toEqual(["c1"]);
  });

  it("orders newest first, ties by id", () => {
    const rows = claudePickerRows(
      [row({ id: "b", modified: 5 }), row({ id: "c", modified: 9 }), row({ id: "a", modified: 5 })],
      "",
    );
    expect(rows.map((r) => r.id)).toEqual(["c", "a", "b"]);
  });

  it("filters by title or folder, ignoring case and padding", () => {
    const items = [
      row({ id: "1", title: "Refactor parser", cwd: "/work/compiler" }),
      row({ id: "2", title: "Docs", cwd: "/work/site" }),
    ];
    expect(claudePickerRows(items, "  PARSER ").map((r) => r.id)).toEqual(["1"]);
    expect(claudePickerRows(items, "site").map((r) => r.id)).toEqual(["2"]);
    expect(claudePickerRows(items, "nothing")).toEqual([]);
  });

  it("marks a linked row with its Forge session", () => {
    const rows = claudePickerRows(
      [row({ id: "l", linked_session_id: "s9", imported_session_id: "s9" }), row({ id: "o", imported_session_id: "s2" })],
      "",
    );
    expect(Object.fromEntries(rows.map((r) => [r.id, r.linkedSessionId]))).toEqual({ l: "s9", o: "s2" });
  });
});

describe("handoff helpers", () => {
  it("quotes only what a shell would split", () => {
    expect(shellQuote("/home/me/api")).toBe("/home/me/api");
    expect(shellQuote("/home/me/my api")).toBe("'/home/me/my api'");
    expect(shellQuote("/it's")).toBe(`'/it'\\''s'`);
  });

  it("builds the copyable command", () => {
    expect(copyableCommand({ cwd: "/p/a b", command: "claude --resume x" })).toBe("cd '/p/a b' && claude --resume x");
  });

  it("types a queued command once", () => {
    queueInitialCommand("s1", "claude", "claude --resume x");
    expect(takeInitialCommand("s1", "term-1")).toBeNull();
    expect(takeInitialCommand("s1", "claude")).toBe("claude --resume x");
    expect(takeInitialCommand("s1", "claude")).toBeNull();
  });

  it("words the continue result", () => {
    expect(continueToast({ mode: "imported", appended: 12 })).toBe("Imported from Claude Code");
    expect(continueToast({ mode: "synced", appended: 0 })).toBe("Already up to date");
    expect(continueToast({ mode: "synced", appended: 1 })).toBe("Synced 1 new turn from Claude Code");
    expect(continueToast({ mode: "synced", appended: 4 })).toBe("Synced 4 new turns from Claude Code");
  });

  it("finds the Claude Code conversation of a Forge session", () => {
    const items = [row({ id: "c1", linked_session_id: "s1" }), row({ id: "c2", imported_session_id: "s2" })];
    expect(linkedClaudeId(items, "s1")).toBe("c1");
    expect(linkedClaudeId(items, "s2")).toBe("c2");
    expect(linkedClaudeId(items, "s3")).toBeNull();
  });
});
