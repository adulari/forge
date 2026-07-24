// Which session the desktop shell chrome (docks, split panes) acts on. The routed session
// wins; off a session route the chrome falls back to the busiest fleet row so ⌘J/⌘G still
// have something to open — the same fallback `UsageDock` already uses for its quota lookup.
import { usePathname } from "expo-router";

import { type SessionRow } from "../../lib/api";
import { useSessions } from "../../lib/queries";

export function routedSessionId(pathname: string): string | null {
  return pathname.match(/^\/session\/([^/]+)/)?.[1] ?? null;
}

export function useActiveSessionId(): string | null {
  const pathname = usePathname();
  const { data } = useSessions();
  return routedSessionId(pathname) ?? data?.find((row) => row.busy)?.id ?? data?.[0]?.id ?? null;
}

export function useSessionRow(sessionId: string | null): SessionRow | null {
  const { data } = useSessions();
  if (!sessionId) return null;
  return data?.find((row) => row.id === sessionId) ?? null;
}
