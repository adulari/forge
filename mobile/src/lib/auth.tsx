// Auth/pairing context. Persists a MULTI-SERVER list (`forge.servers`) + the active server id
// (`forge.activeServerId`) via expo-secure-store (secureStore.web.ts shim on web). Exposes
// `baseUrl` for api.ts/ws.ts/queries.ts (the ACTIVE server's baseUrl — same signature as the
// single-server version, so those files need zero changes) and a probe to validate pairing.
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { getIdentity, probeConnection } from "./api";
import { deleteSecureItem, getSecureItem, setSecureItem } from "./secureStore";
import { parseConnectUrl } from "./connectUrl";
import { applyServerIdentity, reconcileAnywhereHosts, type ManagedAnywhereHost, type StoredServer } from "./serverTargets";
export { parseConnectUrl, type ParsedConnectUrl } from "./connectUrl";
export { type StoredServer } from "./serverTargets";

// Legacy single-server key from before multi-server support; migrated into `forge.servers`
// on first load, then deleted.
const LEGACY_STORAGE_KEY = "forge.connectUrl";
const SERVERS_KEY = "forge.servers";
const ACTIVE_SERVER_KEY = "forge.activeServerId";

export type ConnectTestState =
  | "idle"
  | "testing"
  | "ok"
  | "bad-token"
  | "unreachable"
  | "server-error";


function makeServerId(): string {
  return `srv_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

async function loadServers(): Promise<StoredServer[]> {
  const raw = await getSecureItem(SERVERS_KEY);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as StoredServer[]) : [];
  } catch {
    return [];
  }
}

async function saveServers(servers: StoredServer[]): Promise<void> {
  await setSecureItem(SERVERS_KEY, JSON.stringify(servers));
}

interface AuthContextValue {
  baseUrl: string | null;
  token: string | null;
  host: string | null;
  isLoading: boolean;
  isPaired: boolean;
  servers: StoredServer[];
  activeServerId: string | null;
  /** `setActive` defaults to true — pass false to add a server (e.g. from Settings)
   * without hijacking whatever server the user is currently connected to. */
  addServer: (connectUrl: string, options?: { setActive?: boolean }) => Promise<StoredServer>;
  removeServer: (id: string) => Promise<void>;
  renameServer: (id: string, name: string) => Promise<void>;
  setActive: (id: string) => void;
  syncAnywhereHosts: (hosts: readonly ManagedAnywhereHost[]) => Promise<void>;
  /** @deprecated legacy single-server alias for `addServer` — kept for the old Connect screen. */
  pair: (connectUrl: string) => Promise<void>;
  /** @deprecated legacy single-server alias for `removeServer(activeServerId)`. */
  forget: () => Promise<void>;
  testConnection: (candidateBaseUrl?: string) => Promise<ConnectTestState>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [servers, setServers] = useState<StoredServer[]>([]);
  const [activeServerId, setActiveServerId] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const serverMutationQueue = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        let list = await loadServers();
        let active = await getSecureItem(ACTIVE_SERVER_KEY);

        // One-time migration: fold the old single-server value into the list.
        if (list.length === 0) {
          const legacy = await getSecureItem(LEGACY_STORAGE_KEY);
          if (legacy) {
            const parsed = parseConnectUrl(legacy);
            if (parsed) {
              const server: StoredServer = {
                id: makeServerId(),
                name: parsed.host,
                baseUrl: parsed.baseUrl,
                token: parsed.token,
                host: parsed.host,
                addedAt: Date.now(),
              };
              list = [server];
              active = server.id;
              await saveServers(list);
              await setSecureItem(ACTIVE_SERVER_KEY, active);
            }
          }
          await deleteSecureItem(LEGACY_STORAGE_KEY);
        }

        if (cancelled) return;
        const resolvedActive =
          active && list.some((s) => s.id === active) ? active : (list[0]?.id ?? null);
        setServers(list);
        setActiveServerId(resolvedActive);
      } catch (err) {
        // Fail open: treat a broken read/migration as "no servers" rather than hanging
        // RootNavigator's loading spinner (and the native splash) forever.
        console.warn("[auth] boot load failed, treating as unpaired:", err);
        if (!cancelled) {
          setServers([]);
          setActiveServerId(null);
        }
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const addServer = useCallback(
    async (connectUrl: string, options?: { setActive?: boolean }): Promise<StoredServer> => {
      const parsed = parseConnectUrl(connectUrl);
      if (!parsed) {
        throw new Error("Not a valid Forge connect URL");
      }
      let server: StoredServer = {
        id: makeServerId(),
        name: parsed.host,
        baseUrl: parsed.baseUrl,
        token: parsed.token,
        host: parsed.host,
        addedAt: Date.now(),
      };
      try {
        server = applyServerIdentity(server, await getIdentity(parsed.baseUrl));
      } catch {
        // Older/offline daemons still pair normally; their endpoint remains only a fallback label.
      }
      return enqueueMutation(serverMutationQueue, async () => {
        const current = await loadServers();
        const existing = current.find((item) =>
          item.baseUrl === server.baseUrl || (server.token.length > 0 && item.token === server.token),
        );
        const replacement = existing
          ? {
              ...server,
              id: existing.id,
              addedAt: existing.addedAt,
              ...(existing.customName ? { name: existing.name, customName: true } : {}),
            }
          : server;
        const next = [...current.filter((item) => item.id !== replacement.id), replacement];
        await saveServers(next);
        const shouldActivate = options?.setActive ?? true;
        if (shouldActivate) await setSecureItem(ACTIVE_SERVER_KEY, replacement.id);
        setServers(next);
        if (shouldActivate) setActiveServerId(replacement.id);
        return replacement;
      });
    },
    [],
  );

  const renameServer = useCallback(
    async (id: string, name: string): Promise<void> => {
      const trimmed = name.trim();
      if (!trimmed) throw new Error("Server name cannot be empty");
      await enqueueMutation(serverMutationQueue, async () => {
        const current = await loadServers();
        if (!current.some((server) => server.id === id)) throw new Error("Server is no longer available");
        const next = current.map((server) =>
          server.id === id ? { ...server, name: trimmed, customName: true } : server,
        );
        await saveServers(next);
        setServers(next);
      });
    },
    [],
  );

  const removeServer = useCallback(
    async (id: string): Promise<void> => {
      await enqueueMutation(serverMutationQueue, async () => {
        const current = await loadServers();
        const next = current.filter((s) => s.id !== id);
        await saveServers(next);
        const active = await getSecureItem(ACTIVE_SERVER_KEY);
        let nextActive = active;
        if (active === id) {
          nextActive = next[0]?.id ?? null;
          if (nextActive) await setSecureItem(ACTIVE_SERVER_KEY, nextActive);
          else await deleteSecureItem(ACTIVE_SERVER_KEY);
        }
        setServers(next);
        setActiveServerId(nextActive);
      });
    },
    [],
  );

  const setActive = useCallback(
    (id: string) => {
      if (!servers.some((s) => s.id === id)) return;
      setActiveServerId(id);
      setSecureItem(ACTIVE_SERVER_KEY, id).catch(() => {
        // best-effort persistence; in-memory state already switched
      });
    },
    [servers],
  );

  const syncAnywhereHosts = useCallback(
    async (hosts: readonly ManagedAnywhereHost[]): Promise<void> => {
      await enqueueMutation(serverMutationQueue, async () => {
        // Always merge against protected storage inside the same queue as add/remove. This keeps
        // a cold-start Anywhere reconciliation from publishing an empty, stale React closure.
        const next = await reconcileAnywhereHosts(loadServers, saveServers, hosts);
        setServers(next);
        const active = await getSecureItem(ACTIVE_SERVER_KEY);
        if (active && !next.some((server) => server.id === active)) {
          const fallback = next[0]?.id ?? null;
          if (fallback) await setSecureItem(ACTIVE_SERVER_KEY, fallback);
          else await deleteSecureItem(ACTIVE_SERVER_KEY);
          setActiveServerId(fallback);
        }
      });
    },
    [],
  );

  const pair = useCallback(
    async (connectUrl: string): Promise<void> => {
      await addServer(connectUrl);
    },
    [addServer],
  );

  const forget = useCallback(async (): Promise<void> => {
    if (activeServerId) await removeServer(activeServerId);
  }, [activeServerId, removeServer]);

  const activeServer = useMemo(
    () => servers.find((s) => s.id === activeServerId) ?? null,
    [servers, activeServerId],
  );

  const testConnection = useCallback(
    async (candidateBaseUrl?: string): Promise<ConnectTestState> => {
      const target = candidateBaseUrl ?? activeServer?.baseUrl;
      if (!target) return "unreachable";
      try {
        await probeConnection(target);
        return "ok";
      } catch (err) {
        const status = (err as { status?: number }).status;
        if (status === 404) return "bad-token";
        if (status === 0) return "unreachable";
        return "server-error";
      }
    },
    [activeServer],
  );

  const value = useMemo<AuthContextValue>(
    () => ({
      baseUrl: activeServer?.baseUrl ?? null,
      token: activeServer?.token ?? null,
      host: activeServer?.host ?? null,
      isLoading,
      isPaired: activeServer != null,
      servers,
      activeServerId,
      addServer,
      removeServer,
      renameServer,
      setActive,
      syncAnywhereHosts,
      pair,
      forget,
      testConnection,
    }),
    [
      activeServer,
      isLoading,
      servers,
      activeServerId,
      addServer,
      removeServer,
      renameServer,
      setActive,
      syncAnywhereHosts,
      pair,
      forget,
      testConnection,
    ],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

function enqueueMutation<T>(
  queue: React.MutableRefObject<Promise<void>>,
  operation: () => Promise<T>,
): Promise<T> {
  const result = queue.current.catch(() => undefined).then(operation);
  queue.current = result.then(() => undefined, () => undefined);
  return result;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
