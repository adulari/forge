// The Fleet scope: "All" (every paired server's sessions in one list) or one server (the active
// one). Picking a server also makes it the active server, which is the target for new sessions
// and the server the session screen talks to; "All" leaves the active server as it was.
import { router } from "expo-router";
import { useCallback, useMemo } from "react";

import type { SessionRow } from "./api";
import { useAuth } from "./auth";
import { effectiveShowAll, mergeServerFleets, setStoredShowAll, useStoredShowAll, type ScopedRow } from "./fleetScope";
import { useServerFleets, useSessions } from "./queries";

export type ScopedSessionRow = ScopedRow<SessionRow>;

export function useFleetScope() {
  const { servers, activeServerId, setActive } = useAuth();
  const showAll = effectiveShowAll(useStoredShowAll(), servers.length);
  const selectAll = useCallback(() => setStoredShowAll(true), []);
  const selectServer = useCallback(
    (id: string) => {
      setActive(id);
      setStoredShowAll(false);
    },
    [setActive],
  );
  return { servers, activeServerId, showAll, multiServer: servers.length > 1, selectAll, selectServer };
}

/** The rows the current scope shows, each tagged with the server it came from. */
export function useScopedFleet() {
  const { servers, activeServerId, showAll } = useFleetScope();
  const active = useSessions();
  const fleets = useServerFleets(showAll ? servers : []);
  const fleetData = fleets.map((fleet) => fleet.data);
  const fleetKey = fleets.map((fleet) => fleet.dataUpdatedAt).join(",");

  const rows = useMemo<ScopedSessionRow[]>(() => {
    if (!showAll) return (active.data ?? []).map((row) => ({ row, serverId: activeServerId ?? "" }));
    const data = servers.map((server, index) => (server.id === activeServerId ? (active.data ?? fleetData[index]) : fleetData[index]));
    return mergeServerFleets(servers, data, activeServerId);
    // fleetData is a fresh array each render; fleetKey tracks its contents.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showAll, servers, activeServerId, active.data, fleetKey]);

  const allFailed = showAll && fleets.length > 0 && fleets.every((fleet) => fleet.isError && fleet.data === undefined);
  const refetch = useCallback(
    () => (showAll ? Promise.all(fleets.map((fleet) => fleet.refetch())).then(() => undefined) : active.refetch().then(() => undefined)),
    [showAll, fleets, active],
  );

  return {
    rows,
    showAll,
    isLoading: showAll ? rows.length === 0 && fleets.some((fleet) => fleet.isPending) : active.isLoading,
    isError: showAll ? allFailed : active.isError,
    error: showAll ? fleets.find((fleet) => fleet.error)?.error ?? null : active.error,
    // Last-known rows only exist for the active server (relay hosts paint them before the refetch).
    showingLastKnown: !showAll && (active.data?.length ?? 0) > 0 && active.dataUpdatedAt === 0,
    refetch,
  };
}

/** Open a session on the server of the scope it is rendered in (see `ServerScope`). */
export function useOpenSession() {
  const { activeServerId, setActive } = useAuth();
  return useCallback(
    (sessionId: string) => {
      if (activeServerId) setActive(activeServerId);
      router.push(`/session/${sessionId}`);
    },
    [activeServerId, setActive],
  );
}
