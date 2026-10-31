import type { ListeningPort, PortHost, ProjectAction, RunningAction } from "../../lib/projectActionsModel";

export const ACTION_TERMINAL_PREFIX = "act-";

export const ACTIONS_TOML_SNIPPET = `[[actions]]
name = "dev"
command = "npm run dev"
cwd = "."
icon = "play"`;

export const UNTRUSTED_HINT = "Run `forge trust` in this project to enable its actions";

export function isActionTerminalId(terminalId: string): boolean {
  return terminalId.startsWith(ACTION_TERMINAL_PREFIX);
}

export function actionIdOfTerminal(terminalId: string): string | null {
  return isActionTerminalId(terminalId) ? terminalId.slice(ACTION_TERMINAL_PREFIX.length) : null;
}

export type ActionStatus = "running" | "failed" | "exited" | "idle";

export function actionStatus(action: Pick<ProjectAction, "running" | "exit_code" | "terminal_id">): ActionStatus {
  if (action.running) return "running";
  if (action.terminal_id == null) return "idle";
  return action.exit_code != null && action.exit_code !== 0 ? "failed" : "exited";
}

export function exitBadge(action: Pick<ProjectAction, "running" | "exit_code">): string | null {
  if (action.running || action.exit_code == null || action.exit_code === 0) return null;
  return `exit ${action.exit_code}`;
}

/** The terminal-tab labels for the actions that have a terminal, keyed by terminal id. */
export function actionTerminalNames(actions: readonly ProjectAction[]): Map<string, string> {
  const names = new Map<string, string>();
  for (const action of actions) names.set(`${ACTION_TERMINAL_PREFIX}${action.id}`, action.name);
  return names;
}

export function hostBadge(host: PortHost): { label: string; reachableRemotely: boolean } {
  if (host === "loopback") return { label: "localhost", reachableRemotely: false };
  if (host === "all") return { label: "network", reachableRemotely: true };
  return { label: "bound", reachableRemotely: true };
}

/** Source badge text: the action's name when the port belongs to one of the project's actions. */
export function sourceLabel(port: ListeningPort, actions: readonly ProjectAction[]): string {
  if (port.source === "action") {
    const match = actions.find((action) => action.id === port.action);
    return match?.name ?? port.action ?? "action";
  }
  return port.source;
}

const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "::1", "[::1]", "0.0.0.0"]);

export function hostnameOf(baseUrl: string | null): string | null {
  if (!baseUrl) return null;
  try {
    return new URL(baseUrl).hostname || null;
  } catch {
    return null;
  }
}

export function daemonIsLocal(baseUrl: string | null): boolean {
  const hostname = hostnameOf(baseUrl);
  return hostname == null || LOCAL_HOSTS.has(hostname.toLowerCase());
}

export function localPortUrl(port: number): string {
  return `http://localhost:${port}`;
}

export interface PortLink {
  url: string;
  /** Set when the link probably cannot be reached from this device. */
  note: string | null;
}

/** Where this device should point a browser to reach a port on the daemon's machine. */
export function portLink(port: Pick<ListeningPort, "port" | "host">, baseUrl: string | null): PortLink {
  if (daemonIsLocal(baseUrl)) return { url: localPortUrl(port.port), note: null };
  const hostname = hostnameOf(baseUrl) as string;
  const host = hostname.includes(":") && !hostname.startsWith("[") ? `[${hostname}]` : hostname;
  return {
    url: `http://${host}:${port.port}`,
    note:
      port.host === "loopback"
        ? "This port only listens on the host's loopback, so this device may not be able to reach it."
        : null,
  };
}

export function portsSummary(count: number): string {
  return count === 0 ? "No listening ports" : `${count} listening ${count === 1 ? "port" : "ports"}`;
}

/** Sessions with at least one running action, mapped to those actions' ids. */
export function runningBySession(running: readonly RunningAction[]): Map<string, string[]> {
  const bySession = new Map<string, string[]>();
  for (const row of running) bySession.set(row.session, [...(bySession.get(row.session) ?? []), row.action]);
  return bySession;
}

/** "dev running", "dev, api running" or "3 actions running" for a row's accessible label. */
export function runningLabel(actionIds: readonly string[], names?: ReadonlyMap<string, string>): string {
  if (actionIds.length === 0) return "";
  const labels = actionIds.map((id) => names?.get(id) ?? id);
  if (labels.length > 2) return `${labels.length} actions running`;
  return `${labels.join(", ")} running`;
}

export interface ActionPaletteEntry {
  id: string;
  title: string;
  keywords: string;
  kind: "run" | "stop";
  actionId: string;
}

export function actionPaletteEntries(actions: readonly ProjectAction[]): ActionPaletteEntry[] {
  const entries: ActionPaletteEntry[] = [];
  for (const action of actions) {
    if (action.running) {
      entries.push({
        id: `action-stop-${action.id}`,
        title: `Stop: ${action.name}`,
        keywords: `stop kill action ${action.name} ${action.command}`,
        kind: "stop",
        actionId: action.id,
      });
    } else {
      entries.push({
        id: `action-run-${action.id}`,
        title: `Run: ${action.name}`,
        keywords: `run start action script ${action.name} ${action.command}`,
        kind: "run",
        actionId: action.id,
      });
    }
  }
  return entries;
}
