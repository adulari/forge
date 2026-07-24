import type { AnywhereHost } from "./anywhereApi";
import type { HostState } from "./anywhere/types";

type HostPresence = Pick<AnywhereHost, "online" | "last_heartbeat_at">;
const STALE_AFTER_SECONDS = 90;

export interface ManagedHostPresence {
  heartbeatAgeSec: number;
  state: HostState;
}

export function hostLastActiveMs(host: Pick<AnywhereHost, "last_heartbeat_at">): number | null {
  if (host.last_heartbeat_at === null) return null;
  const seconds = Number(host.last_heartbeat_at);
  return Number.isFinite(seconds) && seconds >= 0 ? seconds * 1000 : null;
}

export function hostStatusText(host: HostPresence): string {
  if (host.online === true) return "Online";
  const lastActiveMs = hostLastActiveMs(host);
  return lastActiveMs === null
    ? "Offline"
    : `Offline · last active ${new Date(lastActiveMs).toLocaleString()}`;
}

export function hostFleetSummary(hosts: readonly Pick<AnywhereHost, "online">[]): string {
  const online = hosts.filter((host) => host.online === true).length;
  return `${online} online · ${hosts.length} ${hosts.length === 1 ? "host" : "hosts"}`;
}

export function managedHostPresence(
  host: HostPresence,
  nowMs: number = Date.now(),
): ManagedHostPresence {
  const lastActiveMs = hostLastActiveMs(host);
  const heartbeatAgeSec = lastActiveMs === null
    ? host.online === true ? 0 : Number.MAX_SAFE_INTEGER
    : Math.max(0, Math.floor((nowMs - lastActiveMs) / 1000));
  if (host.online === true) {
    return { heartbeatAgeSec, state: { kind: "online", activity: "idle" } };
  }
  if (lastActiveMs !== null && heartbeatAgeSec <= STALE_AFTER_SECONDS) {
    return { heartbeatAgeSec, state: { kind: "stale", lastSeenAt: lastActiveMs } };
  }
  return {
    heartbeatAgeSec,
    state: { kind: "offline", lastHeartbeatAt: lastActiveMs ?? 0 },
  };
}
