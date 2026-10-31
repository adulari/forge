// Pure types and helpers for the agent install job API; no React Native imports so vitest can load it.

export type AgentJobState = "queued" | "running" | "done" | "failed";
export type AgentRuntime = "native" | "wsl" | "windows_interop";

export interface AgentInstallJob {
  state: AgentJobState;
  action: "install" | "update" | null;
  lines: string[];
  error: string | null;
  before: string | null;
  after: string | null;
}

export interface AgentInstall {
  id: string;
  name: string;
  installed: boolean;
  version: string | null;
  latest: string | null;
  outdated: boolean;
  runtime: AgentRuntime;
  action: "install" | "update" | null;
  command: string | null;
  manual_hint: string | null;
  job: AgentInstallJob | null;
}

export interface AgentInstallsResponse {
  agents: AgentInstall[];
  outdated: number;
  can_run: boolean;
}

export function isJobActive(job: AgentInstallJob | null | undefined): boolean {
  return job?.state === "queued" || job?.state === "running";
}

export function anyJobActive(agents: AgentInstall[]): boolean {
  return agents.some((agent) => isJobActive(agent.job));
}

export function runtimeLabel(runtime: AgentRuntime): string {
  if (runtime === "wsl") return "WSL";
  if (runtime === "windows_interop") return "Windows via WSL";
  return "native";
}

/** "2.1.314", "2.0.0 → 2.1.0 available", or "not installed". */
export function versionLine(agent: AgentInstall): string {
  if (!agent.installed) return "not installed";
  const version = agent.version ?? "version unknown";
  if (agent.outdated && agent.latest) return `${version} → ${agent.latest} available`;
  if (agent.latest) return `${version} · latest`;
  return version;
}

export function outdatedSummary(count: number): string | null {
  if (count <= 0) return null;
  return count === 1 ? "1 agent is outdated" : `${count} agents are outdated`;
}

export function actionLabel(agent: AgentInstall): string | null {
  if (agent.action === "install") return "Install";
  if (agent.action === "update") {
    if (agent.outdated) return "Update";
    // A CLI known to be current has nothing to offer; an unknown latest still can be re-run.
    return agent.latest ? null : "Update now";
  }
  return null;
}

/** What a finished job should say, for the toast. */
export function jobOutcome(agent: AgentInstall): string | null {
  const job = agent.job;
  if (job?.state === "failed") return job.error ?? `${agent.name} could not be updated.`;
  if (job?.state !== "done") return null;
  const verb = job.action === "install" ? "installed" : "updated";
  return job.after ? `${agent.name} ${verb}: ${job.after}` : `${agent.name} ${verb}.`;
}

/** True when the command downloads a script and pipes it into a shell. */
export function pipesToShell(command: string): boolean {
  return /\|\s*(ba|z)?sh\b/.test(command);
}

/** The agents an "update all" would touch: installed, behind, and with a command to run. */
export function outdatedAgents(agents: AgentInstall[]): AgentInstall[] {
  return agents.filter((agent) => agent.installed && agent.outdated && agent.command);
}

/** The text of the confirmation that stands before any installer runs: the exact commands. */
export function confirmMessage(agents: AgentInstall[]): string {
  const lines = ["Forge will run this on the computer hosting the daemon:", ""];
  for (const agent of agents) {
    lines.push(`${agent.name}`, `$ ${agent.command ?? ""}`);
    if (agent.command && pipesToShell(agent.command)) {
      lines.push("This downloads a script from the internet and runs it.");
    }
  }
  return lines.join("\n");
}
