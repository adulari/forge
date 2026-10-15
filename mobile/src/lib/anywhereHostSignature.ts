import type { AnywhereHost } from "./anywhereApi";

type HostLike = Pick<AnywhereHost, "id" | "name">;

/** Identity of the transports a host list needs: order and array identity do not matter. */
export function hostIdsKey(hosts: readonly Pick<AnywhereHost, "id">[]): string {
  return hosts.map((host) => host.id).sort().join(",");
}

/** Adds display names, which managed-server reconciliation also tracks. */
export function hostSyncKey(hosts: readonly HostLike[]): string {
  return hosts.map((host) => `${host.id}\u0000${host.name}`).sort().join("\u0001");
}

/** Keeps the previous array when a refresh returned the same hosts, so effects keyed on it stay put. */
export function keepIfSameHosts<T extends AnywhereHost>(previous: readonly T[], next: T[]): readonly T[] {
  if (previous.length !== next.length) return next;
  const same = hostSyncKey(previous) === hostSyncKey(next)
    && previous.every((host) => {
      const candidate = next.find((entry) => entry.id === host.id);
      return candidate !== undefined
        && candidate.last_heartbeat_at === host.last_heartbeat_at
        && candidate.device_id === host.device_id;
    });
  return same ? previous : next;
}
