import { describe, expect, it } from "vitest";

import {
  branchNameProblem,
  effectiveWorktreeMode,
  effortForRequest,
  friendlyBranchHint,
  parseSessionDefaults,
  restorableDefaults,
  serializeSessionDefaults,
  sessionDefaultsKey,
  worktreeRequest,
  type DefaultField,
} from "./newSessionDefaults";

describe("sessionDefaultsKey", () => {
  it("is store-safe, per server, and shared between a repo and its worktrees", () => {
    const key = sessionDefaultsKey("srv1", "/home/u/My Project");
    expect(key).toMatch(/^forge\.sessionDefaults\.srv1\.[0-9a-f]{8}$/);
    expect(sessionDefaultsKey("srv2", "/home/u/My Project")).not.toBe(key);
    expect(sessionDefaultsKey("srv1", "/home/u/forge/.forge/worktrees/abc")).toBe(sessionDefaultsKey("srv1", "/home/u/forge/"));
  });
});

describe("parse/serialize", () => {
  it("round-trips and drops unknown or invalid values", () => {
    const saved = { model: "m", temper: "Full", effort: "high", worktree: "new" } as const;
    expect(parseSessionDefaults(serializeSessionDefaults(saved))).toEqual(saved);
    expect(parseSessionDefaults('{"temper":"Chaos","effort":"ultra","worktree":"existing"}')).toEqual({});
    expect(parseSessionDefaults("nope")).toEqual({});
    expect(parseSessionDefaults(null)).toEqual({});
  });
});

describe("restorableDefaults", () => {
  it("never overrides a field the user already touched", () => {
    const saved = { model: "m", temper: "Full", effort: "max" as const, worktree: "off" as const };
    const touched = new Set<DefaultField>(["temper", "effort"]);
    expect(restorableDefaults(saved, touched)).toEqual({ model: "m", worktree: "off" });
    expect(restorableDefaults({}, touched)).toEqual({});
  });
});

describe("worktreeRequest", () => {
  it("maps each mode onto the request", () => {
    expect(worktreeRequest({ mode: "off", cwd: "/r", branch: "x", existingPath: null })).toEqual({ cwd: "/r", worktree: false });
    expect(worktreeRequest({ mode: "new", cwd: "/r", branch: "  ", existingPath: null })).toEqual({ cwd: "/r", worktree: true });
    expect(worktreeRequest({ mode: "new", cwd: "/r", branch: " fix/login ", existingPath: null })).toEqual({ cwd: "/r", worktree: true, branch: "fix/login" });
    expect(worktreeRequest({ mode: "existing", cwd: "/r", branch: "", existingPath: "/r/.forge/worktrees/a" })).toEqual({ cwd: "/r/.forge/worktrees/a", worktree: false });
  });
  it("falls back to off when existing has no pick", () => {
    expect(worktreeRequest({ mode: "existing", cwd: "/r", branch: "", existingPath: null })).toEqual({ cwd: "/r", worktree: false });
  });
});

describe("effectiveWorktreeMode", () => {
  it("forces off for known non-repos and defaults on otherwise", () => {
    expect(effectiveWorktreeMode("new", false, true)).toBe("off");
    expect(effectiveWorktreeMode(null, true, true)).toBe("new");
    expect(effectiveWorktreeMode(null, null, false)).toBe("off");
    expect(effectiveWorktreeMode("existing", true, true)).toBe("existing");
  });
});

describe("branchNameProblem / effort / hint", () => {
  it("validates names", () => {
    expect(branchNameProblem("")).toBeNull();
    expect(branchNameProblem("feat/login-2")).toBeNull();
    expect(branchNameProblem("a b")).toMatch(/spaces/);
    expect(branchNameProblem("a..b")).toMatch(/characters/);
    expect(branchNameProblem("-x")).toMatch(/valid/);
    expect(branchNameProblem("x.lock")).toMatch(/valid/);
  });
  it("maps auto to no effort", () => {
    expect(effortForRequest("auto")).toBeUndefined();
    expect(effortForRequest("max")).toBe("max");
  });
  it("makes a readable hint", () => {
    expect(friendlyBranchHint(0)).toMatch(/^[a-z]+-[a-z]+$/);
  });
});
