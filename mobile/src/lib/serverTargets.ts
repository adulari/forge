export interface StoredServer {
  id: string;
  name: string;
  baseUrl: string;
  token: string;
  host: string;
  addedAt: number;
  /** Missing on legacy rows and therefore equivalent to the unchanged direct transport. */
  transport?: "direct" | "anywhere";
}

export interface ManagedAnywhereHost {
  id: string;
  name: string;
}

/** Pure target reconciliation: direct/LAN rows are byte-for-byte preserved. */
export function mergeAnywhereHosts(
  servers: readonly StoredServer[],
  hosts: readonly ManagedAnywhereHost[],
  addedAt = Date.now(),
): StoredServer[] {
  const direct = servers.filter((server) => server.transport !== "anywhere");
  const existing = new Map(
    servers.filter((server) => server.transport === "anywhere").map((server) => [server.id, server]),
  );
  const managed = hosts.map((host) => {
    const previous = existing.get(`anywhere:${host.id}`);
    return {
      id: `anywhere:${host.id}`,
      name: host.name,
      baseUrl: `fany://${host.id}`,
      token: "",
      host: host.name,
      addedAt: previous?.addedAt ?? addedAt,
      transport: "anywhere" as const,
    };
  });
  return [...direct, ...managed];
}

export async function reconcileAnywhereHosts(
  load: () => Promise<StoredServer[]>,
  save: (servers: StoredServer[]) => Promise<void>,
  hosts: readonly ManagedAnywhereHost[],
): Promise<StoredServer[]> {
  const current = await load();
  const next = mergeAnywhereHosts(current, hosts);
  if (JSON.stringify(next) !== JSON.stringify(current)) await save(next);
  return next;
}
