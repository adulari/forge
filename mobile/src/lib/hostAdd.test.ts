import { describe, expect, it } from "vitest";

import { canAddMachineVia, hostAddCommand, isTerminalPhase, isValidHostTarget } from "./hostAddCore";

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
