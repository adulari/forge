// Keeps every paired host's notices fresh and announces new ones. Triggers: app open, return to
// foreground (or web tab focus), a hosted push arriving or being tapped, a notification tap, and
// a cheap cursor-based poll of the active host while the app is in front. Renders nothing — the
// banner lives in NoticeBannerHost, inside the app lock.
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useCallback, useEffect, useRef } from "react";
import { AppState } from "react-native";

import { getVisibleSessionIds } from "../../lib/alertTargets";
import { haptics } from "../../lib/haptics";
import { isUnseen, osCopy, sessionIdOf, type Notice } from "../../lib/notices/core";
import {
  dismissGenericPushes,
  observeNotifications,
  prepareNotificationChannels,
  presentLocal,
} from "../../lib/notices/localNotify";
import { getNotifyPrefs } from "../../lib/notifyPrefs";
import {
  categoryForPriority,
  channelForPriority,
  isDuplicateAlert,
  rememberAlert,
  routeAlert,
} from "../../lib/notifyPrefsCore";
import {
  forgetNoticeHost,
  getNoticesSnapshot,
  showBanner,
  syncNotices,
} from "../../lib/notices/store";
import { useAuth } from "../../lib/auth";
import { refreshPushRegistration } from "../../lib/push";
import { openSessionFromAlert } from "../../lib/openSessionFromAlert";
import { isTauri, isWeb } from "../../lib/platform";
import { isAppBackgrounded } from "../ds/ToastHost";
import { useOpenNotice } from "./useOpenNotice";

const POLL_MS = 30_000;
const MAX_HOSTS = 8;
const MAX_OS_NOTICES = 3;
const APP_LOCK_KEY = "forge.appLock";

function inBackground(): boolean {
  return isWeb || isTauri ? isAppBackgrounded() : AppState.currentState !== "active";
}

async function appLockOn(): Promise<boolean> {
  try {
    return (await AsyncStorage.getItem(APP_LOCK_KEY)) === "true";
  } catch {
    return true;
  }
}

export function NoticesWatcher() {
  const { servers, activeServerId, isLoading } = useAuth();
  const open = useOpenNotice();
  const serversRef = useRef(servers);
  const activeRef = useRef(activeServerId);
  const openRef = useRef(open);
  const shownAt = useRef<Map<string, number>>(new Map());
  useEffect(() => {
    serversRef.current = servers;
    activeRef.current = activeServerId;
    openRef.current = open;
  });

  /** Resolves to how many real-text OS notifications it raised. */
  const announce = useCallback(async (serverId: string, fresh: Notice[]): Promise<number> => {
    if (fresh.length === 0) return 0;
    const prefs = await getNotifyPrefs();
    const backgrounded = inBackground();
    const now = Date.now();
    const wanted = fresh.filter((notice) => {
      const route = routeAlert({
        category: categoryForPriority(notice.priority),
        prefs,
        sessionId: sessionIdOf(notice),
        visibleSessionIds: getVisibleSessionIds(),
        backgrounded,
      });
      if (!route.toast && !route.os) return false;
      const key = `${serverId}:${notice.title}:${notice.message}`;
      if (isDuplicateAlert(shownAt.current, key, now)) return false;
      rememberAlert(shownAt.current, key, now);
      return true;
    });
    if (wanted.length === 0) return 0;
    showBanner({ serverId, notices: wanted });
    if (!backgrounded) {
      haptics.select();
      return 0;
    }
    const locked = prefs.hideText || (await appLockOn());
    const raised = await Promise.all(
      wanted.slice(0, MAX_OS_NOTICES).map((notice) =>
        presentLocal({
          ...osCopy(notice, locked),
          serverId,
          sessionId: sessionIdOf(notice),
          seq: notice.seq,
          at: notice.at,
          channel: channelForPriority(notice.priority),
          sound: prefs.sound,
        }).catch(() => false),
      ),
    );
    return locked ? 0 : raised.filter(Boolean).length;
  }, []);

  const syncServers = useCallback(
    async (only?: string) => {
      const ordered = [...serversRef.current].sort(
        (a, b) => Number(b.id === activeRef.current) - Number(a.id === activeRef.current),
      );
      const targets = (only ? ordered.filter((s) => s.id === only) : ordered).slice(0, MAX_HOSTS);
      const raised = await Promise.all(
        targets.map(async (server) => announce(server.id, await syncNotices({ id: server.id, baseUrl: server.baseUrl }))),
      );
      void dismissGenericPushes(raised.reduce((sum, n) => sum + n, 0)).catch(() => undefined);
    },
    [announce],
  );

  const serverKey = servers.map((s) => `${s.id}|${s.baseUrl}`).join(",");

  useEffect(() => {
    void prepareNotificationChannels().catch(() => undefined);
  }, []);

  useEffect(() => {
    if (isLoading) return;
    const known = new Set(serversRef.current.map((s) => s.id));
    for (const id of Object.keys(getNoticesSnapshot().hosts)) if (!known.has(id)) forgetNoticeHost(id);
    void syncServers();
  }, [isLoading, serverKey, syncServers]);

  const activeBaseUrl = servers.find((s) => s.id === activeServerId)?.baseUrl ?? null;
  useEffect(() => {
    if (isLoading) return;
    void refreshPushRegistration(activeBaseUrl);
  }, [isLoading, activeBaseUrl]);

  useEffect(() => {
    if (isLoading) return;
    const lifecycle = AppState.addEventListener("change", (state) => {
      if (state === "active") void syncServers();
    });
    const onWebFocus = () => {
      if (typeof document !== "undefined" && document.visibilityState === "visible") void syncServers();
    };
    if (isWeb) {
      document.addEventListener("visibilitychange", onWebFocus);
      window.addEventListener("focus", onWebFocus);
    }
    const poll = setInterval(() => {
      if (AppState.currentState === "active" && !inBackground() && activeRef.current) {
        void syncServers(activeRef.current);
      }
    }, POLL_MS);
    return () => {
      lifecycle.remove();
      clearInterval(poll);
      if (isWeb) {
        document.removeEventListener("visibilitychange", onWebFocus);
        window.removeEventListener("focus", onWebFocus);
      }
    };
  }, [isLoading, syncServers]);

  useEffect(() => {
    if (isLoading) return;
    const onWorkerMessage = (event: MessageEvent) => {
      const data = event.data as { type?: unknown; session?: unknown } | null;
      if (data?.type === "forge:open-session" && typeof data.session === "string") openSessionFromAlert(data.session);
    };
    if (isWeb && typeof navigator !== "undefined" && "serviceWorker" in navigator) {
      navigator.serviceWorker.addEventListener("message", onWorkerMessage);
    }
    const stopObserving = observeNotifications({
      onGeneric: (kind) => {
        void (async () => {
          await syncServers();
          if (kind !== "tap") return;
          // A tapped hosted push names nothing; open the newest notice it stood for.
          let best: { serverId: string; notice: Notice } | null = null;
          for (const [serverId, host] of Object.entries(getNoticesSnapshot().hosts)) {
            const notice = host.items.find((n) => isUnseen(host, n));
            if (notice && (!best || notice.at > best.notice.at)) best = { serverId, notice };
          }
          if (best) {
            showBanner(null);
            openRef.current(best.serverId, best.notice);
          }
        })();
      },
      onNoticeTap: ({ serverId, sessionId, seq, at }) => {
        showBanner(null);
        openRef.current(serverId, {
          seq,
          at,
          title: "",
          message: "",
          priority: "",
          session_id: sessionId ?? "",
        });
      },
    });
    return () => {
      stopObserving();
      if (isWeb && typeof navigator !== "undefined" && "serviceWorker" in navigator) {
        navigator.serviceWorker.removeEventListener("message", onWorkerMessage);
      }
    };
  }, [isLoading, syncServers]);

  return null;
}
