// Sessions from every paired host, so the Inbox badge, the Inbox list and the alerts notice a
// session waiting on a host that is not the active one. The active host keeps its own live query
// (socket-driven); the others share `useServerFleets`' per-host query at a slow poll, which stops
// when the app is backgrounded like every other fleet poll.
import { useMemo } from "react";

import type { SessionRow } from "./api";
import { allHostRows, watchedServers } from "./allHostRows";
import { useAuth } from "./auth";
import type { ScopedSessionRow } from "./useScopedFleet";
import { OTHER_HOST_WATCH_POLL_MS } from "./peekQueries";
import { useServerFleets, useSessions } from "./queries";

export function useAllHostRows(): ScopedSessionRow[] {
  const { servers, activeServerId } = useAuth();
  const active = useSessions();
  const watched = useMemo(() => watchedServers(servers, activeServerId), [servers, activeServerId]);
  const others = useMemo(() => watched.filter((server) => server.id !== activeServerId), [watched, activeServerId]);
  const fleets = useServerFleets(others, OTHER_HOST_WATCH_POLL_MS);
  const fleetKey = fleets.map((fleet) => fleet.dataUpdatedAt).join(",");

  return useMemo(() => {
    const byId = new Map<string, SessionRow[] | undefined>(others.map((server, index) => [server.id, fleets[index]?.data]));
    return allHostRows(watched, activeServerId, active.data, byId);
    // fleets is a fresh array each render; fleetKey tracks its contents.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [watched, others, activeServerId, active.data, fleetKey]);
}
