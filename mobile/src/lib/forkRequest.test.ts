import { describe, expect, it } from "vitest";

import type { SessionRow } from "./api";
import { buildForkRequest, rewindDisabledReason, sessionsSharingDirectory, sharedWorktreeWarning } from "./forkRequest";

const row = (id: string, worktree: string | null, title = id, cwd = "/repo"): SessionRow => ({ id, title, worktree, cwd }) as SessionRow;

describe("buildForkRequest", () => {
  it("sends only the fork point when nothing else is set", () => {
    expect(buildForkRequest({ atSeq: 4, model: null, effort: null, temper: null, prompt: "  ", title: "" })).toEqual({ at_seq: 4 });
  });
  it("asks for the whole thread and a move only when told to", () => {
    expect(
      buildForkRequest({ atSeq: 4, model: null, effort: null, temper: null, prompt: "", title: "", wholeThread: true, moveSource: true }),
    ).toEqual({ at_seq: 4, whole_thread: true, move_source: true });
  });
  it("carries model, effort, temper, prompt and title, trimmed", () => {
    expect(
      buildForkRequest({ atSeq: 2, model: "inferhub::m", effort: "high", temper: "Auto-edit", prompt: " go ", title: " try m " }),
    ).toEqual({ at_seq: 2, model: "inferhub::m", effort: "high", temper: "Auto-edit", prompt: "go", title: "try m" });
  });
});

describe("worktree sharing", () => {
  const sessions = [row("a", "/w/1", "Alpha"), row("b", "/w/1", "Beta"), row("c", "/w/2"), row("d", null)];
  it("finds other sessions on the same worktree", () => {
    expect(sessionsSharingDirectory(sessions, "a").map((s) => s.id)).toEqual(["b"]);
    expect(sessionsSharingDirectory(sessions, "c")).toEqual([]);
    expect(sessionsSharingDirectory(sessions, "d")).toEqual([]);
    expect(sessionsSharingDirectory(undefined, "a")).toEqual([]);
  });
  it("sees a fork child that runs in the worktree without recording it", () => {
    const forked = [row("a", "/w/1", "Alpha", "/w/1"), row("fork", null, "Fork", "/w/1")];
    expect(sessionsSharingDirectory(forked, "a").map((s) => s.id)).toEqual(["fork"]);
    expect(sessionsSharingDirectory(forked, "fork").map((s) => s.id)).toEqual(["a"]);
  });
  it("sees sessions sharing a plain repo cwd, and ignores other directories", () => {
    const plain = [row("a", null, "A", "/repo"), row("b", null, "B", "/repo"), row("c", null, "C", "/other")];
    expect(sessionsSharingDirectory(plain, "a").map((s) => s.id)).toEqual(["b"]);
  });
  it("words the warning by count", () => {
    expect(sharedWorktreeWarning([])).toBeNull();
    expect(sharedWorktreeWarning([row("b", "/w", "Beta")])).toContain("“Beta” is live in this directory");
    expect(sharedWorktreeWarning([row("b", "/w", "B"), row("c", "/w", "C"), row("d", "/w", "D")])).toContain("and 1 more are live");
  });
});

describe("rewindDisabledReason", () => {
  it("blocks only while busy", () => {
    expect(rewindDisabledReason(true)).toMatch(/finish/);
    expect(rewindDisabledReason(false)).toBeNull();
  });
});
