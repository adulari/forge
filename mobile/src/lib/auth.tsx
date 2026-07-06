// Auth/pairing context. Persists the daemon connect URL (which embeds the token as its
// last path segment, per BUILD_PLAN §1.1/§5) via expo-secure-store (secureStore.web.ts
// shim on web). Exposes `baseUrl` for api.ts/ws.ts and a probe to validate pairing.
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

const STORAGE_KEY = "forge.connectUrl";

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

interface AuthContextValue {
  baseUrl: string | null;
  token: string | null;
  host: string | null;
  isLoading: boolean;
  isPaired: boolean;
  pair: (connectUrl: string) => Promise<void>;
  forget: () => Promise<void>;
  testConnection: (
    candidateBaseUrl?: string,
  ) => Promise<ConnectTestState>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [parsed, setParsed] = useState<ParsedConnectUrl | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const stored = await getSecureItem(STORAGE_KEY);
      if (cancelled) return;
      setParsed(stored ? parseConnectUrl(stored) : null);
      setIsLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const pair = useCallback(async (connectUrl: string) => {
    const next = parseConnectUrl(connectUrl);
    if (!next) {
      throw new Error("Not a valid Forge connect URL");
    }
    await setSecureItem(STORAGE_KEY, connectUrl.trim());
    setParsed(next);
  }, []);

  const forget = useCallback(async () => {
    await deleteSecureItem(STORAGE_KEY);
    setParsed(null);
  }, []);

  const testConnection = useCallback(
    async (candidateBaseUrl?: string): Promise<ConnectTestState> => {
      const target = candidateBaseUrl ?? parsed?.baseUrl;
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
    [parsed],
  );

  const value = useMemo<AuthContextValue>(
    () => ({
      baseUrl: parsed?.baseUrl ?? null,
      token: parsed?.token ?? null,
      host: parsed?.host ?? null,
      isLoading,
      isPaired: parsed != null,
      pair,
      forget,
      testConnection,
    }),
    [parsed, isLoading, pair, forget, testConnection],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
