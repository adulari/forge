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
  useState,
} from "react";

import { probeConnection } from "./api";
import { deleteSecureItem, getSecureItem, setSecureItem } from "./secureStore";

// Legacy single-server key from before multi-server support; migrated into `forge.servers`
// on first load, then deleted.
const LEGACY_STORAGE_KEY = "forge.connectUrl";
const SERVERS_KEY = "forge.servers";
const ACTIVE_SERVER_KEY = "forge.activeServerId";

// 32-char hex accepted 16-64 per BUILD_PLAN §1.1.
const TOKEN_RE = /^[0-9a-f]{16,64}$/i;

export type ConnectTestState =
  | "idle"
  | "testing"
  | "ok"
  | "bad-token"
  | "unreachable"
  | "server-error";

export interface ParsedConnectUrl {
  baseUrl: string; // e.g. https://host:port/<token> (no trailing slash)
  token: string;
  host: string;
}

/** Parses a `connect:` URL (or a plain http(s) URL) of shape `{scheme}://{host}:{port}/{token}`. */
export function parseConnectUrl(input: string): ParsedConnectUrl | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  // `forge serve` prints a `connect:` scheme; normalize to http(s) for parsing.
  const normalized = trimmed.replace(/^connect:/i, "https:");

  let url: URL;
  try {
    url = new URL(normalized);
  } catch {
    return null;
  }

  const segments = url.pathname.split("/").filter(Boolean);
  const token = segments[segments.length - 1];
  if (!token || !TOKEN_RE.test(token)) return null;

  const baseUrl = `${url.protocol}//${url.host}/${segments.join("/")}`.replace(/\/$/, "");
  return { baseUrl, token, host: url.host };
}

export interface StoredServer {
  id: string;
  name: string;
  baseUrl: string;
  token: string;
  host: string;
  addedAt: number;
}

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
  addServer: (connectUrl: string) => Promise<StoredServer>;
  removeServer: (id: string) => Promise<void>;
  setActive: (id: string) => void;
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

  useEffect(() => {
    let cancelled = false;
    (async () => {
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
      setIsLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const addServer = useCallback(
    async (connectUrl: string): Promise<StoredServer> => {
      const parsed = parseConnectUrl(connectUrl);
      if (!parsed) {
        throw new Error("Not a valid Forge connect URL");
      }
      const server: StoredServer = {
        id: makeServerId(),
        name: parsed.host,
        baseUrl: parsed.baseUrl,
        token: parsed.token,
        host: parsed.host,
        addedAt: Date.now(),
      };
      // Replace an existing entry for the same baseUrl instead of duplicating it.
      const next = [...servers.filter((s) => s.baseUrl !== server.baseUrl), server];
      await saveServers(next);
      await setSecureItem(ACTIVE_SERVER_KEY, server.id);
      setServers(next);
      setActiveServerId(server.id);
      return server;
    },
    [servers],
  );

  const removeServer = useCallback(
    async (id: string): Promise<void> => {
      const next = servers.filter((s) => s.id !== id);
      await saveServers(next);
      let nextActive = activeServerId;
      if (activeServerId === id) {
        nextActive = next[0]?.id ?? null;
        if (nextActive) await setSecureItem(ACTIVE_SERVER_KEY, nextActive);
        else await deleteSecureItem(ACTIVE_SERVER_KEY);
      }
      setServers(next);
      setActiveServerId(nextActive);
    },
    [servers, activeServerId],
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
      setActive,
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
      setActive,
      pair,
      forget,
      testConnection,
    ],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
