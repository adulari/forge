import { AppState, type AppStateStatus } from "react-native";
import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { isAppBackgrounded, useToast } from "../ds/ToastHost";
import { haptics } from "../../lib/haptics";
import { getVisibleSessionIds } from "../../lib/alertTargets";
import { notify } from "../../lib/notify";
import { getNotifyPrefs } from "../../lib/notifyPrefs";
import {
  alertCopy,
  detectFleetAlerts,
  isDuplicateAlert,
  rememberAlert,
  routeAlert,
  type FleetAlert,
} from "../../lib/notifyPrefsCore";
import { playAlertChime } from "../../lib/notifySound";
import { openSessionFromAlert } from "../../lib/openSessionFromAlert";
import { isTauri, isWeb } from "../../lib/platform";
import { useSessions } from "../../lib/queries";
import type { SessionRow } from "../../lib/api";
import { useAuth } from "../../lib/auth";
import { TWebSocket } from "../../lib/transport";
import { supportsFleetInvalidationSocket } from "../../lib/fleetTransport";
import { useDesktopNativeSync } from "../../lib/desktopMenu";
import {
  clearHostRestarting,
  HOST_GOING_AWAY_CODE,
  isHostRestarting,
  markHostRestarting,
  onHostCap,
} from "../../lib/hostStatus";

const DEBOUNCE_MS = 1200;
const ALERT_TOAST_MS = 6000;
const MAX_ALERT_TOASTS = 3;

const TONE = { done: "success", attention: "warn", error: "danger" } as const;

export function FleetWatcher() {
  // The Tauri tray + native Appearance switch need the same fleet state this component already
  // watches, and they must run inside the provider tree (query client + theme). No-op off desktop.
  useDesktopNativeSync();

  const { data } = useSessions();
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  const toast = useToast();
  // A `[host]` ceiling refusing a session, terminal or helper is a notice, wherever it happened.
  const toastRef = useRef(toast);
  useEffect(() => {
    toastRef.current = toast;
  }, [toast]);
  useEffect(() => onHostCap((message) => toastRef.current.show(message, { tone: "warn" })), []);
  const previous = useRef<Map<string, SessionRow>>(new Map());
  const queued = useRef<Map<string, FleetAlert>>(new Map());
  const shownAt = useRef<Map<string, number>>(new Map());
  const foreground = useRef(true);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!baseUrl || !supportsFleetInvalidationSocket(baseUrl)) return;
    let active = true;
    let socket: WebSocket | null = null;
    let reconnect: ReturnType<typeof setTimeout> | null = null;
    let attempts = 0;

    const connect = () => {
      if (!active || AppState.currentState !== "active") return;
      const url = new URL(`${baseUrl}/ws/fleet`);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      socket = new TWebSocket(url.toString());
      socket.onopen = () => {
        attempts = 0;
        if (isHostRestarting()) {
          clearHostRestarting();
          void queryClient.invalidateQueries({ queryKey: ["sessions", baseUrl] });
        }
      };
      socket.onmessage = (event) => {
        try {
          const frame = JSON.parse(String(event.data)) as { kind?: unknown; revision?: unknown };
          if (frame.kind === "fleet_changed" && typeof frame.revision === "number") {
            void queryClient.invalidateQueries({ queryKey: ["sessions", baseUrl] });
          }
        } catch {
          // Ignore malformed invalidations; the recovery poll remains authoritative.
        }
      };
      socket.onclose = (event?: { code?: number }) => {
        socket = null;
        if (!active || AppState.currentState !== "active") return;
        if (event?.code === HOST_GOING_AWAY_CODE) {
          markHostRestarting();
          toastRef.current.show("host restarting — reconnecting", { tone: "neutral" });
        }
        const delay = isHostRestarting() ? 1_000 : Math.min(15_000, 500 * 2 ** attempts++);
        reconnect = setTimeout(connect, delay);
      };
    };

    const lifecycle = AppState.addEventListener("change", (state) => {
      if (state === "active") connect();
      else {
        if (reconnect) clearTimeout(reconnect);
        reconnect = null;
        socket?.close();
        socket = null;
      }
    });
    connect();
    return () => {
      active = false;
      lifecycle.remove();
      if (reconnect) clearTimeout(reconnect);
      socket?.close();
    };
  }, [baseUrl, queryClient]);

  useEffect(() => {
    const onState = (next: AppStateStatus) => {
      foreground.current = next === "active";
      if (foreground.current) previous.current = new Map();
    };
    const sub = AppState.addEventListener("change", onState);
    return () => {
      sub.remove();
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  useEffect(() => {
    if (!data || !foreground.current) return;
    const old = previous.current;
    previous.current = new Map(data.map((row) => [row.id, row]));
    if (old.size === 0) return;
    for (const alert of detectFleetAlerts(old, data)) queued.current.set(`${alert.sessionId}:${alert.category}`, alert);
    if (queued.current.size === 0 || timer.current) return;
    timer.current = setTimeout(() => {
      timer.current = null;
      const batch = [...queued.current.values()];
      queued.current.clear();
      void raiseAlerts(batch);
    }, DEBOUNCE_MS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data]);

  async function raiseAlerts(batch: FleetAlert[]) {
    const prefs = await getNotifyPrefs();
    const now = Date.now();
    const backgrounded = (isWeb || isTauri) && isAppBackgrounded();
    const toasts: FleetAlert[] = [];
    let sounded = false;
    for (const alert of batch) {
      const route = routeAlert({
        category: alert.category,
        prefs,
        sessionId: alert.sessionId,
        visibleSessionIds: getVisibleSessionIds(),
        backgrounded,
      });
      const key = `${alert.sessionId}:${alert.category}`;
      if (!route.toast && !route.os) continue;
      if (isDuplicateAlert(shownAt.current, key, now)) continue;
      rememberAlert(shownAt.current, key, now);
      if (route.toast) toasts.push(alert);
      if (route.os) {
        const copy = alertCopy(alert, prefs.hideText);
        void notify(copy.title, copy.body, { onClick: () => openSessionFromAlert(alert.sessionId) });
      }
    }
    for (const alert of toasts.slice(0, MAX_ALERT_TOASTS)) {
      toast.show(alertCopy(alert, false).toast, {
        tone: TONE[alert.category],
        duration: ALERT_TOAST_MS,
        system: false,
        action: { label: "Open", onPress: () => openSessionFromAlert(alert.sessionId) },
      });
      sounded = true;
    }
    if (toasts.length > MAX_ALERT_TOASTS) {
      toast.show(`${toasts.length - MAX_ALERT_TOASTS} more sessions need a look`, { tone: "neutral", system: false });
    }
    if (sounded) {
      haptics.select();
      if (prefs.sound) playAlertChime();
    }
  }

  return null;
}
