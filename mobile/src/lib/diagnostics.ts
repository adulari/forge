import type { DiagnosticsResponse } from "./api";
import type { DesktopUpdateState } from "./updater";

export type CompatibilityStatus =
  | "compatible"
  | "version-skew"
  | "daemon-outdated"
  | "client-outdated"
  | "unknown";

export interface Compatibility {
  status: CompatibilityStatus;
  title: string;
  detail: string;
}

export function assessCompatibility(
  daemonProtocol: number | undefined,
  daemonVersion: string | undefined,
  clientProtocol: number,
  clientVersion: string,
): Compatibility {
  if (daemonProtocol == null) {
    return {
      status: "unknown",
      title: "Compatibility unknown",
      detail: "This daemon is too old to report its protocol version.",
    };
  }
  if (daemonProtocol > clientProtocol) {
    return {
      status: "client-outdated",
      title: "App update required",
      detail: `The daemon speaks protocol v${daemonProtocol}; this app supports v${clientProtocol}.`,
    };
  }
  if (daemonProtocol < clientProtocol) {
    return {
      status: "daemon-outdated",
      title: "Daemon update required",
      detail: `This app speaks protocol v${clientProtocol}; the daemon supports v${daemonProtocol}.`,
    };
  }
  if (daemonVersion && daemonVersion !== clientVersion) {
    return {
      status: "version-skew",
      title: "Compatible version skew",
      detail: `App v${clientVersion} and daemon v${daemonVersion} share protocol v${clientProtocol}.`,
    };
  }
  return {
    status: "compatible",
    title: "App and daemon are compatible",
    detail: `Both speak protocol v${clientProtocol}.`,
  };
}

const SAFE_CHECKS: Record<string, string> = {
  database: "Session database",
  terminal: "Terminal runtime",
  config: "Layered configuration",
  git: "Git",
};

/**
 * Build a support summary from a strict whitelist. It intentionally excludes hostname,
 * connection URLs/tokens, workspace data, logs, prompts, and all daemon-provided free text.
 */
export function buildSupportSummary(
  diagnostics: DiagnosticsResponse,
  clientVersion: string,
  clientProtocol: number,
  update: DesktopUpdateState,
  nativeRuntimeVersion: string | null = null,
): string {
  const { host, resources, runtime } = diagnostics;
  const checks = diagnostics.checks
    .filter((check) => SAFE_CHECKS[check.id])
    .map((check) => `${SAFE_CHECKS[check.id]}=${check.status === "ok" ? "ok" : "warn"}`)
    .join(", ");
  return [
    "Forge sanitized support summary",
    `client=${clientVersion} protocol=${clientProtocol} native_runtime=${nativeRuntimeVersion ?? "unavailable"}`,
    `daemon=${host.version} protocol=${host.protocol}`,
    `platform=${host.os}/${host.arch} pid=${host.pid} uptime_s=${host.process_uptime_secs}`,
    `process_memory_b=${resources.process_memory_bytes} process_virtual_b=${resources.process_virtual_memory_bytes}`,
    `system_memory_b=${resources.system_available_memory_bytes}/${resources.system_total_memory_bytes} cpus=${resources.cpu_count}`,
    `load=${resources.load_average_one.toFixed(2)}/${resources.load_average_five.toFixed(2)}/${resources.load_average_fifteen.toFixed(2)}`,
    `sessions=${runtime.sessions} busy=${runtime.busy_sessions} waiting=${runtime.waiting_sessions}`,
    `terminals=${runtime.terminals} terminal_clients=${runtime.terminal_clients}`,
    `push=web:${runtime.web_push_ready ? "ready" : "off"},native:${runtime.native_push_ready ? "ready" : "off"}`,
    `checks=${checks || "none"}`,
    `desktop_update=${update.phase}${update.availableVersion ? `:${update.availableVersion}` : ""}`,
    `checked_at=${diagnostics.checked_at}`,
  ].join("\n");
}
