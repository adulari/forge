import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

import type { StoredServer } from "./serverTargets";

/** What the Fleet tells you about one paired machine, beyond "reachable or not". */
export type HostBadge = "selected" | "kept-ready" | "connected" | "offline" | "disabled";

export const HOST_BADGE_LABEL: Record<HostBadge, string> = {
  selected: "Selected",
  "kept-ready": "Kept ready",
  connected: "Connected",
  offline: "Offline",
  disabled: "Disabled",
};

const RECENT_LIMIT = 6;

export interface PresenceState {
  /** Last time each server answered, by server id. */
  lastSeen: Record<string, number>;
  /** Server ids, most recently selected first. */
  recent: string[];
}

export const EMPTY_PRESENCE: PresenceState = { lastSeen: {}, recent: [] };

export function enabledServers<T extends { disabled?: boolean }>(servers: readonly T[]): T[] {
  return servers.filter((server) => !server.disabled);
}

/**
 * The one other host kept ready: the most recently selected enabled server that is not the
 * active one. Its fleet stays polled in a single-server scope so switching to it is instant and
 * its waiting count never goes stale.
 */
/** One entry per host that answered its latest poll, stamped with when it answered. Keyed on the
 * poll time rather than on who is reachable, so every successful poll refreshes last-seen. */
export function seenUpdates(
  servers: readonly { id: string }[],
  fleets: readonly { isSuccess: boolean; dataUpdatedAt?: number }[],
): { id: string; at: number }[] {
  const seen: { id: string; at: number }[] = [];
  servers.forEach((server, index) => {
    const fleet = fleets[index];
    if (fleet?.isSuccess && fleet.dataUpdatedAt) seen.push({ id: server.id, at: fleet.dataUpdatedAt });
  });
  return seen;
}

/** Only an enabled, known server can be made active. */
export function canActivate(servers: readonly { id: string; disabled?: boolean }[], id: string): boolean {
  return servers.some((server) => server.id === id && !server.disabled);
}

/** The server to fall back to when the active one goes away: never a disabled one. */
export function firstEnabledId(servers: readonly { id: string; disabled?: boolean }[]): string | null {
  return servers.find((server) => !server.disabled)?.id ?? null;
}

export function pickWarmServer<T extends { id: string; disabled?: boolean }>(
  servers: readonly T[],
  activeId: string | null,
  recent: readonly string[],
): T | null {
  for (const id of recent) {
    if (id === activeId) continue;
    const found = servers.find((server) => server.id === id && !server.disabled);
    if (found) return found;
  }
  return null;
}

export function hostBadge(args: {
  id: string;
  activeId: string | null;
  warmId: string | null;
  disabled: boolean;
  reachable: boolean;
}): HostBadge {
  if (args.disabled) return "disabled";
  if (!args.reachable) return "offline";
  if (args.id === args.activeId) return "selected";
  if (args.id === args.warmId) return "kept-ready";
  return "connected";
}

/** "last seen 4m ago", or "never seen from this device" when no poll ever succeeded. */
export function lastSeenLabel(lastSeenMs: number | undefined, nowMs: number = Date.now()): string {
  if (!lastSeenMs) return "not seen yet";
  const seconds = Math.max(0, Math.round((nowMs - lastSeenMs) / 1000));
  if (seconds < 60) return "last seen just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `last seen ${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `last seen ${hours}h ago`;
  return `last seen ${Math.round(hours / 24)}d ago`;
}

export function recordSelected(state: PresenceState, id: string): PresenceState {
  if (state.recent[0] === id) return state;
  return { ...state, recent: [id, ...state.recent.filter((item) => item !== id)].slice(0, RECENT_LIMIT) };
}

/** Writes at most once a minute per host: a last-seen time needs no finer grain. */
export function recordSeen(state: PresenceState, id: string, nowMs: number): PresenceState {
  const previous = state.lastSeen[id];
  if (previous !== undefined && nowMs - previous < 60_000) return state;
  return { ...state, lastSeen: { ...state.lastSeen, [id]: nowMs } };
}

export function pruneRemoved(state: PresenceState, servers: readonly { id: string }[]): PresenceState {
  const ids = new Set(servers.map((server) => server.id));
  const seenIds = Object.keys(state.lastSeen);
  if (seenIds.every((id) => ids.has(id)) && state.recent.every((id) => ids.has(id))) return state;
  return {
    lastSeen: Object.fromEntries(Object.entries(state.lastSeen).filter(([id]) => ids.has(id))),
    recent: state.recent.filter((id) => ids.has(id)),
  };
}

export function setServerDisabled<T extends StoredServer>(servers: readonly T[], id: string, disabled: boolean): T[] {
  return servers.map((server) => {
    if (server.id !== id || Boolean(server.disabled) === disabled) return server;
    const { disabled: _previous, ...rest } = server;
    return (disabled ? { ...rest, disabled: true } : rest) as T;
  });
}

const STORAGE_KEY = "forge.hostPresence.v1";
let state: PresenceState = EMPTY_PRESENCE;
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(next: PresenceState): void {
  if (next === state) return;
  state = next;
  for (const listener of listeners) listener();
  AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next)).catch(() => undefined);
}

function ensureLoaded(): Promise<void> {
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => {
      if (!raw) return;
      const parsed = JSON.parse(raw) as Partial<PresenceState>;
      const loaded: PresenceState = {
        lastSeen: { ...(parsed.lastSeen ?? {}), ...state.lastSeen },
        recent: [...state.recent, ...(parsed.recent ?? []).filter((id) => !state.recent.includes(id))].slice(
          0,
          RECENT_LIMIT,
        ),
      };
      state = loaded;
      for (const listener of listeners) listener();
    })
    .catch(() => undefined);
  return hydration;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

export function usePresence(): PresenceState {
  return useSyncExternalStore(subscribe, () => state, () => state);
}

export function markSelected(id: string): void {
  void ensureLoaded().then(() => emit(recordSelected(state, id)));
}

export function markSeen(id: string, nowMs: number = Date.now()): void {
  void ensureLoaded().then(() => emit(recordSeen(state, id, nowMs)));
}

export function forgetRemoved(servers: readonly { id: string }[]): void {
  void ensureLoaded().then(() => emit(pruneRemoved(state, servers)));
}
