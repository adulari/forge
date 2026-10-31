import { describe, expect, it } from "vitest";

import {
  actionLabel,
  anyJobActive,
  confirmMessage,
  outdatedAgents,
  pipesToShell,
  jobOutcome,
  outdatedSummary,
  runtimeLabel,
  versionLine,
  type AgentInstall,
} from "./agentInstallsCore";

const base: AgentInstall = {
  id: "claude-cli",
  name: "Claude Code",
  installed: true,
  version: "2.0.0",
  latest: "2.1.0",
  outdated: true,
  runtime: "native",
  action: "update",
  command: "claude update",
  manual_hint: null,
  job: null,
};

describe("agent install helpers", () => {
  it("shows installed vs latest", () => {
    expect(versionLine(base)).toBe("2.0.0 → 2.1.0 available");
    expect(versionLine({ ...base, outdated: false, latest: "2.0.0" })).toBe("2.0.0 · latest");
    expect(versionLine({ ...base, outdated: false, latest: null })).toBe("2.0.0");
    expect(versionLine({ ...base, installed: false, version: null })).toBe("not installed");
  });

  it("summarises how many are outdated", () => {
    expect(outdatedSummary(0)).toBeNull();
    expect(outdatedSummary(1)).toBe("1 agent is outdated");
    expect(outdatedSummary(3)).toBe("3 agents are outdated");
  });

  it("labels actions and runtimes", () => {
    expect(actionLabel(base)).toBe("Update");
    expect(actionLabel({ ...base, outdated: false, latest: "1.0.0" })).toBeNull();
    expect(actionLabel({ ...base, outdated: false, latest: null })).toBe("Update now");
    expect(actionLabel({ ...base, installed: false, action: "install" })).toBe("Install");
    expect(actionLabel({ ...base, action: null })).toBeNull();
    expect(runtimeLabel("wsl")).toBe("WSL");
    expect(runtimeLabel("windows_interop")).toBe("Windows via WSL");
    expect(runtimeLabel("native")).toBe("native");
  });

  it("reports job outcomes and activity", () => {
    const job = { state: "done" as const, action: "update" as const, lines: [], error: null, before: "2.0.0", after: "2.1.0" };
    expect(jobOutcome({ ...base, job })).toBe("Claude Code updated: 2.1.0");
    expect(jobOutcome({ ...base, job: { ...job, state: "failed", error: "exited with 1" } })).toBe("exited with 1");
    expect(jobOutcome(base)).toBeNull();
    expect(anyJobActive([base, { ...base, job: { ...job, state: "running" } }])).toBe(true);
    expect(anyJobActive([{ ...base, job }])).toBe(false);
  });

  it("shows the exact command before anything runs", () => {
    const install = { ...base, installed: false, outdated: false, command: "curl -fsSL https://claude.ai/install.sh | bash" };
    const text = confirmMessage([install]);
    expect(text).toContain("$ curl -fsSL https://claude.ai/install.sh | bash");
    expect(text).toContain("downloads a script");
    expect(confirmMessage([base])).not.toContain("downloads a script");
    expect(pipesToShell("npm install -g @openai/codex@latest")).toBe(false);
    expect(outdatedAgents([base, { ...base, id: "codex-cli", outdated: false }, { ...base, id: "x", installed: false }])).toHaveLength(1);
  });
});
