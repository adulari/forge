// Types and response parsing for project actions and ports; no network code, so tests can import it.
export interface ProjectAction {
  id: string;
  name: string;
  command: string;
  cwd: string | null;
  icon: string | null;
  terminal_id: string | null;
  running: boolean;
  exit_code: number | null;
}

export interface ProjectActionsResponse {
  actions: ProjectAction[];
  /** The project declares actions but is not trusted, so the daemon hides them. */
  untrusted: boolean;
}

export interface RunningAction {
  session: string;
  action: string;
}

export type PortHost = "loopback" | "all" | "other";
export type PortSource = "action" | "terminal" | "workspace";

export interface ListeningPort {
  port: number;
  host: PortHost;
  pid: number;
  process: string;
  command: string;
  source: PortSource;
  action?: string;
}

const asString = (value: unknown): string => (typeof value === "string" ? value : "");
const asNullableString = (value: unknown): string | null => (typeof value === "string" && value ? value : null);

export function parseProjectActions(raw: unknown): ProjectActionsResponse {
  const body = (raw ?? {}) as { actions?: unknown; untrusted?: unknown };
  const rows = Array.isArray(body.actions) ? body.actions : [];
  const actions = rows
    .map((row): ProjectAction | null => {
      const item = (row ?? {}) as Record<string, unknown>;
      const id = asString(item.id);
      if (!id) return null;
      return {
        id,
        name: asString(item.name) || id,
        command: asString(item.command),
        cwd: asNullableString(item.cwd),
        icon: asNullableString(item.icon),
        terminal_id: asNullableString(item.terminal_id),
        running: item.running === true,
        exit_code: typeof item.exit_code === "number" ? item.exit_code : null,
      };
    })
    .filter((row): row is ProjectAction => row != null);
  return { actions, untrusted: body.untrusted === true };
}

export function parseRunningActions(raw: unknown): RunningAction[] {
  const rows = (raw as { running?: unknown } | null)?.running;
  if (!Array.isArray(rows)) return [];
  return rows
    .map((row) => row as Partial<RunningAction> | null)
    .filter((row): row is RunningAction => typeof row?.session === "string" && typeof row.action === "string");
}

export function parsePorts(raw: unknown): ListeningPort[] {
  const rows = (raw as { ports?: unknown } | null)?.ports;
  if (!Array.isArray(rows)) return [];
  return rows
    .map((row): ListeningPort | null => {
      const item = (row ?? {}) as Record<string, unknown>;
      if (typeof item.port !== "number" || !Number.isInteger(item.port) || item.port < 1 || item.port > 65535) return null;
      const host: PortHost = item.host === "all" || item.host === "other" ? item.host : "loopback";
      const source: PortSource = item.source === "action" || item.source === "terminal" ? item.source : "workspace";
      return {
        port: item.port,
        host,
        pid: typeof item.pid === "number" ? item.pid : 0,
        process: asString(item.process),
        command: asString(item.command),
        source,
        action: asNullableString(item.action) ?? undefined,
      };
    })
    .filter((row): row is ListeningPort => row != null)
    .sort((a, b) => a.port - b.port);
}
