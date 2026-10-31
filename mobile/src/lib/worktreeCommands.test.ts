import { describe, expect, it } from "vitest";

import { pullResultView, worktreeNewCommand, worktreeSetupBanner } from "./worktreeCommands";

const base = { message: "", conflicts: [], handoff_prompt: null, stashed: false };

describe("worktreeNewCommand", () => {
  it("builds the slash command with an optional name and --carry", () => {
    expect(worktreeNewCommand("", false)).toBe("/worktree new");
    expect(worktreeNewCommand("  fix-login ", true)).toBe("/worktree new fix-login --carry");
    expect(worktreeNewCommand("", true)).toBe("/worktree new --carry");
  });
});

describe("pullResultView", () => {
  it("only offers the handoff on conflicts that carry a prompt", () => {
    expect(pullResultView({ ...base, status: "conflict", conflicts: ["a.rs"], handoff_prompt: "resolve" }).canHandoff).toBe(true);
    expect(pullResultView({ ...base, status: "conflict" }).canHandoff).toBe(false);
    expect(pullResultView({ ...base, status: "stash_conflict", handoff_prompt: "x" }).canHandoff).toBe(true);
    expect(pullResultView({ ...base, status: "updated", handoff_prompt: "x" }).canHandoff).toBe(false);
  });
  it("words each outcome", () => {
    expect(pullResultView({ ...base, status: "up_to_date" }).title).toBe("Already up to date");
    expect(pullResultView({ ...base, status: "updated", stashed: true }).title).toMatch(/re-applied/);
    expect(pullResultView({ ...base, status: "failed" }).tone).toBe("danger");
  });
});

describe("worktreeSetupBanner", () => {
  it("shows running, a failure's tail, interruption and a dismissible done", () => {
    expect(worktreeSetupBanner(null)).toBeNull();
    expect(worktreeSetupBanner({ state: "running", message: "" })?.dismissible).toBe(false);
    const failed = worktreeSetupBanner({ state: "failed", message: "npm ERR! code 1" });
    expect(failed?.tone).toBe("warn");
    expect(failed?.message).toContain("npm ERR! code 1");
    expect(failed?.expandable).toBe(false);
    expect(worktreeSetupBanner({ state: "interrupted", message: "daemon stopped" })?.message).toContain("incomplete");
    expect(worktreeSetupBanner({ state: "done", message: "setup ran" })?.message).toContain("setup ran");
  });

  it("keeps the END of long output and previews the last three lines", () => {
    const output = Array.from({ length: 50 }, (_, i) => `line ${i}`).join("\n");
    const banner = worktreeSetupBanner({ state: "failed", message: output });
    expect(banner?.preview).toContain("line 49");
    expect(banner?.preview).toContain("line 47");
    expect(banner?.preview).not.toContain("line 10");
    expect(banner?.preview.split("\n").length).toBeLessThanOrEqual(3);
    expect(banner?.preview.endsWith("line 49")).toBe(true);
    expect(banner?.expandable).toBe(true);
    const huge = worktreeSetupBanner({ state: "failed", message: `START${"x".repeat(9000)}END` });
    expect(huge?.message).toContain("END");
    expect(huge?.message).not.toContain("START");
  });
});
