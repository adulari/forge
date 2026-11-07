// What the Ship and Pull requests screens may do on this connection. A direct connection can do
// everything. Over Forge Anywhere the host bridges the PR reads and, only when its owner switched
// on `[anywhere] allow_ship_over_relay`, ship and PR Watch writes. The host says which in the
// `ship` field of `GET /api/identity`; a host that predates the bridged routes omits it.
import { useQuery } from "@tanstack/react-query";

import { getIdentity, type HostIdentity } from "./api";
import { useAuth } from "./auth";
import { supportsDirectDaemonEndpoints } from "./transport";

import { SHIP_OFF_MESSAGE, UPDATE_HOST_MESSAGE } from "./prAccessMessages";

export { SHIP_OFF_MESSAGE, UPDATE_HOST_MESSAGE };

export type PrAccess =
  | { kind: "none" }
  | { kind: "direct" }
  | { kind: "relay"; state: "checking" | "update-host" | "ready"; shipEnabled: boolean };

export function prAccessFor(
  baseUrl: string | null,
  identity: Pick<HostIdentity, "ship"> | undefined,
  checking: boolean,
): PrAccess {
  if (!baseUrl) return { kind: "none" };
  if (supportsDirectDaemonEndpoints(baseUrl)) return { kind: "direct" };
  if (identity?.ship?.pr) return { kind: "relay", state: "ready", shipEnabled: identity.ship.over_relay === true };
  return { kind: "relay", state: checking ? "checking" : "update-host", shipEnabled: false };
}

/** PR reads, PR Watch reads and the merge queue work: direct, or a bridged host that reports it. */
export function prReadable(access: PrAccess): boolean {
  return access.kind === "direct" || (access.kind === "relay" && access.state === "ready");
}

/** Whether Ship and PR Watch writes may be sent, and the sentence to show when they may not. */
export function shipGate(access: PrAccess): { allowed: boolean; message: string | null } {
  if (access.kind === "direct") return { allowed: true, message: null };
  if (access.kind !== "relay") return { allowed: false, message: null };
  if (access.state === "update-host") return { allowed: false, message: UPDATE_HOST_MESSAGE };
  if (access.state === "checking") return { allowed: false, message: null };
  return access.shipEnabled ? { allowed: true, message: null } : { allowed: false, message: SHIP_OFF_MESSAGE };
}

export function usePrAccess(): PrAccess {
  const { baseUrl } = useAuth();
  const relay = baseUrl != null && !supportsDirectDaemonEndpoints(baseUrl);
  const identity = useQuery({
    queryKey: ["host", "ship-capability", baseUrl],
    queryFn: () => getIdentity(baseUrl as string),
    enabled: relay,
    staleTime: 60_000,
  });
  return prAccessFor(baseUrl ?? null, identity.data, identity.isPending);
}

/** The base URL PR reads go to, or null when this connection cannot read pull requests. */
export function usePrReadBase(): string | null {
  const { baseUrl } = useAuth();
  const access = usePrAccess();
  return baseUrl && prReadable(access) ? baseUrl : null;
}

/** The base URL Ship and PR Watch writes go to, or null when they are not allowed. */
export function useShipBase(): string | null {
  const { baseUrl } = useAuth();
  const access = usePrAccess();
  return baseUrl && shipGate(access).allowed ? baseUrl : null;
}

export function usePrAvailable(): boolean {
  return prReadable(usePrAccess());
}

export function useShipGate(): { allowed: boolean; message: string | null } {
  return shipGate(usePrAccess());
}

/** Direct connections only: pull request create, comment and merge, and GitHub Actions. */
export function useDirectOnly(): boolean {
  return usePrAccess().kind === "direct";
}
