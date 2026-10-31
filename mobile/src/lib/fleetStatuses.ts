// How reachable each paired server is, read off its fleet poll. Pure so the rule is tested.
import type { SessionRow } from "./api";
import type { HostBadge } from "./hostPresence";

export type ServerReachability = "pending" | "reachable" | "unreachable" | "disabled";

export interface ServerFleetStatus {
  id: string;
  name: string;
  reachability: ServerReachability;
  waiting: number;
  waitingIds: string[];
  /** When this device last got an answer from the host; filled in by the hook that owns presence. */
  lastSeenMs?: number;
  badge?: HostBadge;
}

interface FleetQueryState {
  data?: SessionRow[];
  isPending: boolean;
  isSuccess: boolean;
  failureCount: number;
  /** The last poll failed. A failed poll keeps the data it replaced, so this plus
   * `dataUpdatedAt` is what tells an outage from a blip. */
  isError?: boolean;
  dataUpdatedAt?: number;
}

/** How long a host may go without a good answer before it counts as unreachable. */
const UNREACHABLE_AFTER_MS = 20_000;

export function fleetStatuses(
  servers: readonly { id: string; name: string }[],
  fleets: readonly FleetQueryState[],
  nowMs: number = Date.now(),
): ServerFleetStatus[] {
  return servers.map((server, index) => {
    const fleet = fleets[index];
    // One failed poll is a blip, not an outage: it takes a stretch without a good answer.
    // (`failureCount` cannot say: polls do not retry, so it never passes 1.)
    const staleFor = nowMs - (fleet.dataUpdatedAt ?? nowMs);
    const reachable = fleet.isSuccess || (fleet.data !== undefined && !(fleet.isError && staleFor > UNREACHABLE_AFTER_MS));
    const waitingIds = (fleet.data ?? []).filter((row) => row.waiting).map((row) => row.id);
    return {
      id: server.id,
      name: server.name,
      reachability: fleet.isPending ? "pending" : reachable ? "reachable" : "unreachable",
      waiting: waitingIds.length,
      waitingIds,
    };
  });
}

