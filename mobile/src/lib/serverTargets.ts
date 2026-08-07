export interface StoredServer {
  id: string;
  name: string;
  baseUrl: string;
  token: string;
  host: string;
  addedAt: number;
  /** Missing on legacy rows and therefore equivalent to the unchanged direct transport. */
  transport?: "direct" | "anywhere";
  /** True only after the user explicitly chooses a display name. */
  customName?: boolean;
}

export interface ServerIdentity {
  hostname: string;
}

/** Apply daemon identity without ever confusing the transport endpoint with display identity. */
export function applyServerIdentity(server: StoredServer, identity: ServerIdentity): StoredServer {
  if (server.customName) return server;
  const hostname = identity.hostname.trim();
  return hostname && hostname !== server.name ? { ...server, name: hostname } : server;
}

/** The parts of a detected local daemon needed to recognise a server row that points at it. */
export interface DetectedDaemon {
  base_url: string;
  token: string;
}

/**
 * Repoint stored direct servers whose daemon has moved.
 *
 * A cloudflared quick tunnel gets a new random hostname on every start, so a stored direct URL
 * rots the moment the daemon restarts and every paired client is orphaned with no signal beyond
 * "unreachable". The daemon's token is stable across restarts while its address is not, so a row
 * holding the same token is the same daemon at a new address — safe to repoint, and the only
 * thing that makes reconnection automatic instead of a manual re-paste.
 *
 * Deliberately narrow: only `direct` rows, only an exact token match, and the array identity is
 * preserved when nothing changed so callers can skip a pointless write.
 */
export function repointMovedDaemons(
  servers: readonly StoredServer[],
  detected: DetectedDaemon | null,
): readonly StoredServer[] {
  if (!detected?.token || !detected.base_url) return servers;
  let changed = false;
  const next = servers.map((server) => {
    const direct = (server.transport ?? "direct") === "direct";
    if (!direct || server.token !== detected.token || server.baseUrl === detected.base_url) {
      return server;
    }
    changed = true;
    return { ...server, baseUrl: detected.base_url };
  });
  return changed ? next : servers;
}

export interface ManagedAnywhereHost {
  id: string;
  name: string;
}

/** Hosts not already represented by the canonical managed server target list. */
export function unrepresentedAnywhereHosts<T extends ManagedAnywhereHost>(
  servers: readonly StoredServer[],
  hosts: readonly T[],
): T[] {
  const represented = new Set(
    servers
      .filter((server) => server.transport === "anywhere")
      .map((server) => server.id.replace(/^anywhere:/, "")),
  );
  return hosts.filter((host) => !represented.has(host.id));
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
      name: previous?.customName ? previous.name : host.name,
      baseUrl: `fany://${host.id}`,
      token: "",
      host: host.name,
      addedAt: previous?.addedAt ?? addedAt,
      transport: "anywhere" as const,
      ...(previous?.customName ? { customName: true } : {}),
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
