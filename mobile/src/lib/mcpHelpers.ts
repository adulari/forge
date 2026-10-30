import type { McpServerRow, McpStatus } from "./api";

export interface McpProbe {
  name: string;
  status: McpStatus;
  last_error: string | null;
  tools: { name: string; description: string }[];
  resources: { uri: string; name: string }[];
  disabled_tools: string[];
}

export type McpStatusTone = "success" | "danger" | "warn" | "neutral";

const STATUS: Record<McpStatus, { label: string; tone: McpStatusTone }> = {
  connected: { label: "Connected", tone: "success" },
  failed: { label: "Failed", tone: "danger" },
  unauthorized: { label: "Needs sign-in", tone: "warn" },
  reconnecting: { label: "Reconnecting", tone: "warn" },
  slow: { label: "Slow", tone: "warn" },
  blocked: { label: "Blocked", tone: "warn" },
  disabled: { label: "Disabled", tone: "neutral" },
  unknown: { label: "Not checked", tone: "neutral" },
};

export function mcpStatusMeta(status: McpStatus | undefined): { label: string; tone: McpStatusTone } {
  return STATUS[status ?? "unknown"] ?? STATUS.unknown;
}

export function mcpToolSummary(server: Pick<McpServerRow, "tool_count" | "disabled_tools">): string | null {
  if (server.tool_count == null) return null;
  const off = server.disabled_tools?.length ?? 0;
  const noun = server.tool_count === 1 ? "tool" : "tools";
  return off > 0 ? `${server.tool_count} ${noun} · ${off} off` : `${server.tool_count} ${noun}`;
}

export function toggleToolDisabled(disabled: readonly string[], tool: string, enabled: boolean): string[] {
  const rest = disabled.filter((name) => name !== tool);
  return enabled ? rest : [...rest, tool].sort();
}

export function joinArgs(args: readonly string[] | undefined): string {
  return (args ?? []).map((arg) => (/\s/.test(arg) ? JSON.stringify(arg) : arg)).join(" ");
}

export function splitArgs(text: string): string[] {
  return (text.match(/"(?:[^"\\]|\\.)*"|\S+/g) ?? []).map((part) => {
    if (!part.startsWith('"')) return part;
    try {
      return JSON.parse(part) as string;
    } catch {
      return part;
    }
  });
}

