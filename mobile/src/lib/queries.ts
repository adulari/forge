// react-query v5 hooks over api.ts. Screens call ONLY these hooks (UI_RULES.md #3) —
// never raw fetch. Query keys are namespaced by baseUrl so switching a paired server
// never serves stale cross-server data from the persisted cache.
import {
  useInfiniteQuery,
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useIsFocused } from "expo-router";
import { useEffect, useRef, useState } from "react";

import {
  answer as apiAnswer,
  archiveSession,
  type CreateSessionRequest,
  createSession,
  discardSession,
  getHistory,
  getPastSessions,
  getSessions,
  type HistoryRow,
  mergeSession,
  type PastSessionRow,
  type SessionRow,
  subscribePush,
  uploadFile,
} from "./api";
import { useAuth } from "./auth";
import type { Snapshot } from "./ws";
import { syncWidgetSessions } from "./widgetData";
import {
  endLiveActivity,
  startLiveActivity,
  updateLiveActivity,
} from "../../modules/live-activity";

const SESSIONS_POLL_MS = 3000;
const SERVER_FLEET_POLL_MS = 5000;
const SERVER_FLEET_BACKOFF_MS = 15000;
const PAST_PAGE_SIZE = 50;
const HISTORY_PAGE_SIZE = 60;

function keys(baseUrl: string | null) {
  return {
    sessions: ["sessions", baseUrl] as const,
    pastSessions: ["sessions", "past", baseUrl] as const,
    history: (sessionId: string) => ["history", baseUrl, sessionId] as const,
  };
}

/** Live fleet list. Polls every 3s while the screen is focused (UI_RULES.md perf budget). */
export function useSessions() {
  const { baseUrl } = useAuth();
  const isFocused = useIsFocused();
  const query = useQuery<SessionRow[]>({
    queryKey: keys(baseUrl).sessions,
    queryFn: () => getSessions(baseUrl as string),
    enabled: baseUrl != null,
    refetchInterval: isFocused ? SESSIONS_POLL_MS : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: true,
  });
  useEffect(() => {
    if (query.data) syncWidgetSessions(query.data);
  }, [query.data]);
  return query;
}

/** Per-server fleet probes for the Settings server switcher. */
export function useServerFleets(servers: { id: string; baseUrl: string }[]) {
  return useQueries({
    queries: servers.map((server) => ({
      queryKey: ["sessions", "server", server.id] as const,
      queryFn: () => getSessions(server.baseUrl),
      refetchInterval: (query: { state: { error: unknown } }) =>
        query.state.error ? SERVER_FLEET_BACKOFF_MS : SERVER_FLEET_POLL_MS,
      refetchIntervalInBackground: false,
      retry: false,
    })),
  });
}
/** Past (archived/finished) sessions, infinite by `before` = last row's last_activity. */
export function usePastSessions() {
  const { baseUrl } = useAuth();
  return useInfiniteQuery<PastSessionRow[]>({
    queryKey: keys(baseUrl).pastSessions,
    queryFn: ({ pageParam }) =>
      getPastSessions(baseUrl as string, {
        limit: PAST_PAGE_SIZE,
        before: pageParam as number | undefined,
      }),
    enabled: baseUrl != null,
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (lastPage) =>
      lastPage.length < PAST_PAGE_SIZE
        ? undefined
        : lastPage[lastPage.length - 1]?.last_activity,
  });
}

/** Transcript history for a session, infinite upward by `before` = oldest seq. */
export function useHistory(sessionId: string | null) {
  const { baseUrl } = useAuth();
  return useInfiniteQuery<HistoryRow[]>({
    queryKey: keys(baseUrl).history(sessionId ?? ""),
    queryFn: ({ pageParam }) =>
      getHistory(baseUrl as string, {
        session: sessionId as string,
        limit: HISTORY_PAGE_SIZE,
        before: pageParam as number | undefined,
      }),
    enabled: baseUrl != null && sessionId != null,
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (lastPage) =>
      lastPage.length < HISTORY_PAGE_SIZE
        ? undefined
        : lastPage[lastPage.length - 1]?.seq,
  });
}

export function useCreateSession() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateSessionRequest) =>
      createSession(baseUrl as string, body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).sessions });
    },
  });
}

export function useArchiveSession() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => archiveSession(baseUrl as string, id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).sessions });
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).pastSessions });
    },
  });
}

export function useMergeSession() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => mergeSession(baseUrl as string, id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).sessions });
    },
  });
}

export function useDiscardSession() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => discardSession(baseUrl as string, id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).sessions });
    },
  });
}

export function useAnswer() {
  const { baseUrl } = useAuth();
  return useMutation({
    mutationFn: (body: { session: string; seq: number; allow: boolean }) =>
      apiAnswer(baseUrl as string, body),
  });
}

export function useUpload() {
  const { baseUrl } = useAuth();
  return useMutation({
    mutationFn: ({ sessionId, form }: { sessionId: string; form: FormData }) =>
      uploadFile(baseUrl as string, sessionId, form),
  });
}

/**
 * Fires the history-invalidation rule from ARCHITECTURE.md §4.1.4: on a turn's `busy`
 * true→false edge, invalidate that session's `useHistory` query so the finalized turn
 * appears from the store instead of the (now-stale) streaming/transcript fields. Returns
 * `true` on the render where the edge was detected, so the session shell can also react
 * (e.g. haptic/toast) without wiring its own busy-tracking ref.
 *
 * Same effect also drives this session's Live Activity (iOS-only, no-ops elsewhere): starts
 * one on the idle->busy edge, updates it on every subsequent snapshot change while a turn is
 * running, and ends it on the busy->idle edge — reusing this hook's existing busy-edge
 * detection rather than tracking it a second time elsewhere.
 */
export function useTurnCompleted(snapshot: Snapshot | null): boolean {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  const prevBusyRef = useRef<boolean | undefined>(undefined);
  const activityIdRef = useRef<string | null>(null);
  const [completed, setCompleted] = useState(false);

  const busy = snapshot?.busy ?? false;
  const waiting = snapshot != null && (snapshot.permission_prompt != null || snapshot.question != null);
  const sessionId = snapshot?.session_id ?? null;
  const title = snapshot?.title ?? "";
  const costUsd = snapshot?.cost_usd ?? 0;
  const contextTokens = snapshot?.context_tokens ?? 0;

  // Detect the busy true->false edge in an effect (never read the ref during render).
  useEffect(() => {
    const wasBusy = prevBusyRef.current;
    const didComplete = wasBusy === true && !busy;
    const didBegin = wasBusy === false && busy;
    prevBusyRef.current = busy;
    setCompleted(didComplete);
    if (didComplete && sessionId) {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).history(sessionId) });
    }

    if (!sessionId) return;
    if (didBegin) {
      startLiveActivity(sessionId, title, busy, waiting, costUsd, contextTokens).then(
        ({ activityId, pushToken }) => {
          activityIdRef.current = activityId;
          if (activityId && pushToken && baseUrl) {
            subscribePush(baseUrl, {
              session_id: sessionId,
              push_token: pushToken,
              environment: __DEV__ ? "sandbox" : "production",
            }).catch(() => {});
          }
        },
      );
    } else if (activityIdRef.current) {
      if (didComplete) {
        const id = activityIdRef.current;
        activityIdRef.current = null;
        endLiveActivity(id).catch(() => {});
      } else {
        updateLiveActivity(activityIdRef.current, busy, waiting, costUsd, contextTokens).catch(
          () => {},
        );
      }
    }
  }, [busy, waiting, sessionId, title, costUsd, contextTokens, baseUrl, queryClient]);

  return completed;
}
