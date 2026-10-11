// Per-host notices store. Text lives in memory only (the daemon is the source of truth and
// re-serves its bounded inbox); what persists, in AsyncStorage, are the two marks per host —
// where we last alerted and where the user last looked — so a relaunch neither replays old
// banners nor forgets what was read.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useSyncExternalStore } from "react";

import { getNotices } from "../api";
import {
  emptyHost,
  markSeen as markHostSeen,
  parsePersisted,
  syncHost,
  unseenCount,
  type HostNotices,
  type Notice,
  type PersistedState,
} from "./core";

const STORAGE_KEY = "forge.notices.v1";

export interface NoticeTarget {
  id: string;
  baseUrl: string;
}

export interface BannerState {
  serverId: string;
  notices: Notice[];
}

interface Snapshot {
  hosts: Readonly<Record<string, HostNotices>>;
  banner: BannerState | null;
}

let snapshot: Snapshot = { hosts: {}, banner: null };
let persisted: PersistedState = {};
let hydration: Promise<void> | null = null;
const listeners = new Set<() => void>();
const inFlight = new Map<string, Promise<Notice[]>>();

function publish(next: Snapshot) {
  snapshot = next;
  for (const listener of listeners) listener();
}

function hydrate(): Promise<void> {
  hydration ??= AsyncStorage.getItem(STORAGE_KEY)
    .then((raw) => {
      persisted = parsePersisted(raw);
    })
    .catch(() => undefined);
  return hydration;
}

function persist() {
  const marks: PersistedState = { ...persisted };
  for (const [id, host] of Object.entries(snapshot.hosts)) {
    marks[id] = { notified: host.notified, seen: host.seen };
  }
  persisted = marks;
  void AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(marks)).catch(() => undefined);
}

function hostFor(serverId: string): HostNotices {
  return snapshot.hosts[serverId] ?? emptyHost(persisted[serverId]);
}

function setHost(serverId: string, host: HostNotices) {
  publish({ ...snapshot, hosts: { ...snapshot.hosts, [serverId]: host } });
  persist();
}

/** Fetch the host's new notices. Resolves to the announce-worthy ones; a failed fetch (host
 * offline, relay not ready) resolves to none — the next trigger simply tries again. */
export function syncNotices(target: NoticeTarget, nowSecs = Math.floor(Date.now() / 1000)): Promise<Notice[]> {
  const pending = inFlight.get(target.id);
  if (pending) return pending;
  const run = (async () => {
    await hydrate();
    try {
      const result = await syncHost(hostFor(target.id), (after) => getNotices(target.baseUrl, after), nowSecs);
      setHost(target.id, result.host);
      return result.fresh;
    } catch {
      return [];
    }
  })().finally(() => inFlight.delete(target.id));
  inFlight.set(target.id, run);
  return run;
}

export function showBanner(banner: BannerState | null) {
  publish({ ...snapshot, banner });
}

/** Mark a host's notices seen, optionally only up to one notice. */
export function markNoticesSeen(serverId: string, through?: Notice) {
  const host = snapshot.hosts[serverId];
  if (!host) return;
  setHost(serverId, markHostSeen(host, through));
}

/** Drop a removed server's state. */
export function forgetNoticeHost(serverId: string) {
  if (!snapshot.hosts[serverId] && !persisted[serverId]) return;
  const { [serverId]: _hosts, ...hosts } = snapshot.hosts;
  const { [serverId]: _marks, ...marks } = persisted;
  persisted = marks;
  publish({ ...snapshot, hosts });
  persist();
}

export function getNoticesSnapshot(): Snapshot {
  return snapshot;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useNoticesStore(): Snapshot {
  return useSyncExternalStore(subscribe, getNoticesSnapshot, getNoticesSnapshot);
}

export function totalUnseen(hosts: Snapshot["hosts"]): number {
  return Object.values(hosts).reduce((sum, host) => sum + unseenCount(host), 0);
}
