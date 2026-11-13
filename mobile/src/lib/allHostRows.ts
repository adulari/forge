import { mergeServerFleets, type MergeableRow, type ScopedRow } from "./fleetScope";

/** Hosts whose waiting sessions count: every enabled one, plus the active one even if disabled. */
export function watchedServers<T extends { id: string; disabled?: boolean }>(servers: readonly T[], activeServerId: string | null): T[] {
  return servers.filter((server) => !server.disabled || server.id === activeServerId);
}

/** One list over every watched host. The active host's rows come from its own live query, the
 * others from the shared per-host poll; a daemon paired twice shows once (active wins). */
export function allHostRows<R extends MergeableRow>(
  servers: readonly { id: string }[],
  activeServerId: string | null,
  activeRows: readonly R[] | undefined,
  otherRows: ReadonlyMap<string, readonly R[] | undefined>,
): ScopedRow<R>[] {
  const data = servers.map((server) => (server.id === activeServerId ? activeRows : otherRows.get(server.id)));
  return mergeServerFleets(servers, data, activeServerId);
}

export function waitingScoped<R extends MergeableRow>(rows: readonly ScopedRow<R>[]): ScopedRow<R>[] {
  return rows.filter((item) => item.row.waiting);
}
