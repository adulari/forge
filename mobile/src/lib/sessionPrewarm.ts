// Desktop keeps the last few sessions mounted (hidden) so switching back is a visibility flip.
// This module is the two small seams that feature needs outside the session layout: whether the
// feature is on, and a tiny store through which the rail asks the layout to mount a session it
// is hovering before the click lands.
import { router } from "expo-router";
import { useSyncExternalStore } from "react";
import { Platform } from "react-native";

import { useBreakpoint } from "../theme/useBreakpoint";

/** Sessions kept mounted at once (the open one included). */
export const MAX_KEPT_SESSIONS = 3;

/** Wide web/desktop windows only: phones and narrow windows stack sessions as screens and keep
 * Back, so they must not hold extra mounted sessions. */
export function useKeepSessionsMounted(): boolean {
  const { isExpanded } = useBreakpoint();
  return Platform.OS === "web" && isExpanded;
}

let warmRequest: { id: string } | null = null;
const listeners = new Set<() => void>();

/** Ask the session layout to mount `id` hidden (no-op when the feature is off). */
export function requestSessionPrewarm(id: string): void {
  warmRequest = { id };
  for (const listener of listeners) listener();
}

export function subscribeSessionPrewarm(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

// Activation: a click on a session in the rail makes it the open session in the SAME frame (local
// state in HostedSessions); the URL follows from an idle callback. A router navigation re-renders
// the root navigator, sidebar, palette and session host and was most of what a switch cost once the
// sessions themselves stayed mounted.
let activationOverride: string | null = null;
let activationPending: string | null = null;
let activationHandle: ReturnType<typeof setTimeout> | null = null;
const lastPaths = new Map<string, string>();
const activationListeners = new Set<() => void>();

function notifyActivation(): void {
  for (const listener of activationListeners) listener();
}

/** The path a session was last shown at (its tab included), so reopening it lands where it was. */
export function rememberSessionPath(id: string, path: string): void {
  lastPaths.set(id, path);
}

export function activateSession(id: string): void {
  activationOverride = id;
  activationPending = id;
  notifyActivation();
  if (activationHandle != null) clearTimeout(activationHandle);
  const run = () => {
    activationHandle = null;
    const target = activationPending;
    if (target == null) return;
    // Cleared first: a navigation that lands later must not look like an unfinished request.
    activationPending = null;
    router.replace((lastPaths.get(target) ?? `/session/${target}`) as never);
  };
  const w = window as Window & { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number };
  if (w.requestIdleCallback) w.requestIdleCallback(run, { timeout: 400 });
  else activationHandle = setTimeout(run, 120);
}

export function getActivationOverride(): string | null {
  return activationOverride;
}

/** Called by the host when the URL has caught up with (or moved away from) the override. */
export function settleActivation(routedId: string | null): void {
  if (activationOverride == null) return;
  if (routedId === activationOverride) {
    activationPending = null;
    activationOverride = null;
    notifyActivation();
  } else if (activationPending == null) {
    activationOverride = null;
    notifyActivation();
  }
}

function subscribeActivation(listener: () => void): () => void {
  activationListeners.add(listener);
  return () => {
    activationListeners.delete(listener);
  };
}

/** The session the user just chose, before the URL says so (null once it does). */
export function useSessionActivationOverride(): string | null {
  return useSyncExternalStore(subscribeActivation, getActivationOverride, getActivationOverride);
}

export function getSessionPrewarmRequest(): { id: string } | null {
  return warmRequest;
}
