import type { AnywhereHost } from "./anywhereApi";

type HostPresence = Pick<AnywhereHost, "online" | "last_heartbeat_at">;

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
