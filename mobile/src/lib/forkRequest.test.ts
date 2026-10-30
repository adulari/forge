import { describe, expect, it } from "vitest";

import type { SessionRow } from "./api";
import { buildForkRequest, rewindDisabledReason, sessionsSharingWorktree, sharedWorktreeWarning } from "./forkRequest";

const row = (id: string, worktree: string | null, title = id): SessionRow => ({ id, title, worktree }) as SessionRow;

describe("buildForkRequest", () => {
  it("sends only the fork point when nothing else is set", () => {
    expect(buildForkRequest({ atSeq: 4, model: null, effort: null, temper: null, prompt: "  ", title: "" })).toEqual({ at_seq: 4 });
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
    expect(sessionsSharingWorktree(sessions, "a").map((s) => s.id)).toEqual(["b"]);
    expect(sessionsSharingWorktree(sessions, "c")).toEqual([]);
    expect(sessionsSharingWorktree(sessions, "d")).toEqual([]);
    expect(sessionsSharingWorktree(undefined, "a")).toEqual([]);
  });
  it("words the warning by count", () => {
    expect(sharedWorktreeWarning([])).toBeNull();
    expect(sharedWorktreeWarning([row("b", "/w", "Beta")])).toContain("“Beta” is live in this worktree");
    expect(sharedWorktreeWarning([row("b", "/w", "B"), row("c", "/w", "C"), row("d", "/w", "D")])).toContain("and 1 more are live");
  });
});

describe("rewindDisabledReason", () => {
  it("blocks only while busy", () => {
    expect(rewindDisabledReason(true)).toMatch(/finish/);
    expect(rewindDisabledReason(false)).toBeNull();
  });
});
