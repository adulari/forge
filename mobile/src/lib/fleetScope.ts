import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

/** The fields of a fleet row the merge reads. */
export interface MergeableRow {
  id: string;
  waiting: boolean;
  busy: boolean;
  last_activity: number;
}

export interface ScopedRow<R extends MergeableRow> {
  row: R;
  serverId: string;
}

function stateRank(row: MergeableRow): number {
  return row.waiting ? 0 : row.busy ? 1 : 2;
}

/**
 * One fleet built from every server's list, for the "All" scope.
 *
 * The same daemon is often paired twice (a LAN/direct row and its Anywhere relay row), so a
 * session id seen on more than one server is kept once, from the preferred server when it has
 * it (the active one, so opening the row does not switch servers), otherwise from the first
 * server in list order. Each daemon already sorts waiting-first; the merged list keeps that
 * order across servers (waiting, then busy, then idle, most recent first) because the fleet
 * deck emits one section label per consecutive state run.
 */
export function mergeServerFleets<R extends MergeableRow>(
  servers: readonly { id: string }[],
  fleets: readonly (readonly R[] | undefined)[],
  preferredServerId: string | null,
): ScopedRow<R>[] {
  const order = servers.map((server, index) => ({ server, rows: fleets[index] ?? [] }));
  order.sort((a, b) => Number(b.server.id === preferredServerId) - Number(a.server.id === preferredServerId));
  const seen = new Set<string>();
  const merged: ScopedRow<R>[] = [];
  for (const { server, rows } of order) {
    for (const row of rows) {
      if (seen.has(row.id)) continue;
      seen.add(row.id);
      merged.push({ row, serverId: server.id });
    }
  }
  return merged.sort(
    (a, b) => stateRank(a.row) - stateRank(b.row) || b.row.last_activity - a.row.last_activity,
  );
}

/** "All" only means something with a second server to show; one server is always its own scope. */
export function effectiveShowAll(showAll: boolean, serverCount: number): boolean {
  return showAll && serverCount > 1;
}

const STORAGE_KEY = "forge.fleetScope.v1";

let showAllState = true;
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function setState(next: boolean): void {
  if (next === showAllState) return;
  showAllState = next;
  for (const listener of listeners) listener();
}

function ensureLoaded(): Promise<void> {
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => setState(raw !== "server"))
    .catch(() => undefined);
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

function snapshot(): boolean {
  return showAllState;
}

/** The stored scope choice: true = every server's sessions, false = the active server's only. */
export function useStoredShowAll(): boolean {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function setStoredShowAll(showAll: boolean): void {
  void ensureLoaded().then(() => {
    setState(showAll);
    AsyncStorage.setItem(STORAGE_KEY, showAll ? "all" : "server").catch(() => undefined);
  });
}
