// Signals that come from the host itself rather than from a request: it is restarting, it refused
// something because of a configured ceiling, or its session store needs recovering. Kept in one
// tiny module-level store so a socket handler, the API layer and a screen can all see them.

import { useSyncExternalStore } from "react";

/** WebSocket close code a daemon sends when it is about to restart ("going away"). */
export const HOST_GOING_AWAY_CODE = 1001;
/** Application close code a daemon answers a refused terminal attach with. */
export const HOST_CAP_CLOSE_CODE = 4429;
/** Stable `code` of an error body the daemon sends when a `[host]` ceiling refused a request. */
export const HOST_CAP_ERROR_CODE = "host_cap";
/** Stable `code` of the 503 every route answers while the daemon's session store is unavailable. */
export const STORE_RECOVERY_ERROR_CODE = "store_recovery";

/** How long after a restart notice the app keeps retrying quickly and calls it a restart. */
export const HOST_RESTART_GRACE_MS = 120_000;
export const HOST_RESTART_RETRY_MS = 1_000;

type Listener = () => void;

function createSignal<T>(initial: T) {
  let value = initial;
  const listeners = new Set<Listener>();
  return {
    get: () => value,
    set(next: T) {
      if (Object.is(next, value)) return;
      value = next;
      listeners.forEach((listener) => listener());
    },
    subscribe(listener: Listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

// --- host restarting -------------------------------------------------------------------------

const restartingSince = createSignal<number | null>(null);
let restartExpiry: ReturnType<typeof setTimeout> | null = null;

export function markHostRestarting(now: number = Date.now()): void {
  restartingSince.set(now);
  if (restartExpiry != null) clearTimeout(restartExpiry);
  restartExpiry = setTimeout(() => {
    restartExpiry = null;
    restartingSince.set(null);
  }, HOST_RESTART_GRACE_MS);
}

export function clearHostRestarting(): void {
  if (restartExpiry != null) {
    clearTimeout(restartExpiry);
    restartExpiry = null;
  }
  restartingSince.set(null);
}

export function isHostRestarting(now: number = Date.now()): boolean {
  const since = restartingSince.get();
  return since != null && now - since < HOST_RESTART_GRACE_MS;
}

export function useHostRestarting(): boolean {
  return useSyncExternalStore(
    restartingSince.subscribe,
    () => restartingSince.get() != null,
    () => false,
  );
}

// --- host ceilings ---------------------------------------------------------------------------

type CapListener = (message: string) => void;
const capListeners = new Set<CapListener>();
let lastCap: { message: string; at: number } | null = null;
const CAP_DEDUPE_MS = 2_000;

/** Announce that the host refused something because a `[host]` ceiling is reached. */
export function reportHostCap(message: string, now: number = Date.now()): void {
  if (lastCap && lastCap.message === message && now - lastCap.at < CAP_DEDUPE_MS) return;
  lastCap = { message, at: now };
  capListeners.forEach((listener) => listener(message));
}

export function onHostCap(listener: CapListener): () => void {
  capListeners.add(listener);
  return () => {
    capListeners.delete(listener);
  };
}

// --- store recovery --------------------------------------------------------------------------

const recoveryHost = createSignal<string | null>(null);

/** The daemon at `baseUrl` answered that its session store needs recovering. */
export function reportStoreRecovery(baseUrl: string): void {
  recoveryHost.set(baseUrl);
}

export function clearStoreRecovery(): void {
  recoveryHost.set(null);
}

export function getStoreRecovery(): string | null {
  return recoveryHost.get();
}

export function useStoreRecovery(): string | null {
  return useSyncExternalStore(recoveryHost.subscribe, recoveryHost.get, () => null);
}

/** Look at a failed response for a condition the host is reporting about itself. */
export function noteHostResponse(baseUrl: string, status: number, body: unknown): void {
  const record = body != null && typeof body === "object" ? (body as { code?: unknown; error?: unknown }) : null;
  if (!record) return;
  if (status === 503 && record.code === STORE_RECOVERY_ERROR_CODE) {
    reportStoreRecovery(baseUrl);
  } else if (record.code === HOST_CAP_ERROR_CODE) {
    reportHostCap(typeof record.error === "string" && record.error !== "" ? record.error : "The host reached a limit set in its config.");
  }
}
