import { describe, expect, it } from "vitest";

import {
  canAddMachineVia,
  hostAddCommand,
  isTerminalPhase,
  isValidHostTarget,
  jobToResume,
  nextPollDelay,
  type AddHostJob,
} from "./hostAddCore";

describe("isValidHostTarget", () => {
  it("accepts ssh-style targets and refuses anything ssh or a shell could misread", () => {
    expect(isValidHostTarget("dev@build-box.example.com")).toBe(true);
    expect(isValidHostTarget("[::1]")).toBe(true);
    for (const bad of ["", "  ", "-oProxyCommand=x", "a b", "a;b", "a$(x)", "a'b"]) {
      expect(isValidHostTarget(bad)).toBe(false);
    }
  });
});

describe("hostAddCommand", () => {
  it("builds the terminal equivalent", () => {
    expect(hostAddCommand(" dev@box ", "")).toBe("forge host add dev@box");
    expect(hostAddCommand("dev@box", 'my "box"')).toBe('forge host add dev@box --name "my \\"box\\""');
    expect(hostAddCommand("", "")).toBe("forge host add user@host");
  });
});

describe("phases and transport", () => {
  it("knows when polling can stop", () => {
    expect(isTerminalPhase("done")).toBe(true);
    expect(isTerminalPhase("failed")).toBe(true);
    expect(isTerminalPhase("awaiting_approval")).toBe(false);
  });
  it("only direct daemons can run it", () => {
    expect(canAddMachineVia(undefined)).toBe(true);
    expect(canAddMachineVia("anywhere")).toBe(false);
  });
});

describe("polling and re-attaching", () => {
  it("backs off after failed polls instead of stopping", () => {
    expect(nextPollDelay(0)).toBe(1500);
    expect(nextPollDelay(1)).toBeGreaterThan(1500);
    expect(nextPollDelay(50)).toBe(15_000);
  });
  const job = (id: string, target: string, phase: AddHostJob["phase"]): AddHostJob => ({
    id,
    target,
    phase,
    step: "",
    lines: [],
    safety_code: null,
    remote_safety_code: null,
    error: null,
    host: null,
  });
  it("resumes the running job for the same target after the sheet was closed", () => {
    const jobs = [job("a", "u@one", "done"), job("b", "u@two", "awaiting_approval")];
    expect(jobToResume(jobs, "u@two")?.id).toBe("b");
    expect(jobToResume(jobs, "u@one")).toBeNull();
    expect(jobToResume(jobs)?.id).toBe("b");
  });
});
