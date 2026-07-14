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
  type SessionTreeRow,
  getSessionTree,
  type HookRow,
  getHooks,
  type SkillRow,
  getSkills,
  type ModelsResponse,
  getModels,
  type ConfigResponse,
  type UpdateConfigRequest,
  getConfig,
  updateConfig,
  type PlanRow,
  getPlans,


  type CreateMcpServerRequest,
  createMcpServer,
  type McpResponse,
  getMcp,

  type ForkSessionRequest,
  forkSession,
  answer as apiAnswer,
  archiveSession,
  type CreateSessionRequest,
  createSession,
  discardSession,
  getHistory,
  getUsage,
  type UsageResponse,
  getPastSessions,
  getSessions,
  type HistoryRow,
  mergeSession,
  type PastSessionRow,
  type SessionRow,
  subscribePush,
  unsubscribePush,
  transcribeAudio,
  uploadFile,
} from "./api";
import { useAuth } from "./auth";
import type { Snapshot } from "./ws";
import { syncWidgetSessions } from "./widgetData";
import {
  addLiveActivityPushTokenListener,
  endLiveActivity,
  startLiveActivity,
  updateLiveActivity,
} from "../../modules/live-activity";

const SESSIONS_POLL_MS = 3000;
const SERVER_FLEET_POLL_MS = 5000;
const SERVER_FLEET_BACKOFF_MS = 15000;
const PAST_PAGE_SIZE = 50;
const HISTORY_PAGE_SIZE = 60;
const baselines = new Map<string, number>();

function keys(baseUrl: string | null) {
  return {
    sessions: ["sessions", baseUrl] as const,
    sessionTree: ["sessions", "tree", baseUrl] as const,
    pastSessions: ["sessions", "past", baseUrl] as const,
    history: (sessionId: string) => ["history", baseUrl, sessionId] as const,
    config: ["config", baseUrl] as const,
    skills: ["skills", baseUrl] as const,
    models: ["models", baseUrl] as const,
    hooks: ["hooks", baseUrl] as const,
    plans: ["plans", baseUrl] as const,

    mcp: ["mcp", baseUrl] as const,
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
  const isFocused = useIsFocused();
  return useQueries({
    queries: servers.map((server) => ({
      queryKey: ["sessions", "server", server.id] as const,
      queryFn: () => getSessions(server.baseUrl),
      refetchInterval: isFocused
        ? (query: { state: { error: unknown } }) =>
            query.state.error ? SERVER_FLEET_BACKOFF_MS : SERVER_FLEET_POLL_MS
        : false,
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

export function useSessionTree() { const { baseUrl } = useAuth(); return useQuery<SessionTreeRow[]>({ queryKey: keys(baseUrl).sessionTree, queryFn: () => getSessionTree(baseUrl as string), enabled: baseUrl != null }); }
export function useSkills() { const { baseUrl } = useAuth(); return useQuery<SkillRow[]>({ queryKey: keys(baseUrl).skills, queryFn: () => getSkills(baseUrl as string), enabled: baseUrl != null }); }
export function useHooks() { const { baseUrl } = useAuth(); return useQuery<HookRow[]>({ queryKey: keys(baseUrl).hooks, queryFn: () => getHooks(baseUrl as string), enabled: baseUrl != null }); }
export function useModels() { const { baseUrl } = useAuth(); const isFocused = useIsFocused(); return useQuery<ModelsResponse>({ queryKey: keys(baseUrl).models, queryFn: () => getModels(baseUrl as string), enabled: baseUrl != null, refetchOnWindowFocus: isFocused }); }
export function usePlans() { const { baseUrl } = useAuth(); const isFocused = useIsFocused(); return useQuery<PlanRow[]>({ queryKey: keys(baseUrl).plans, queryFn: () => getPlans(baseUrl as string), enabled: baseUrl != null, refetchInterval: isFocused ? 30000 : false }); }

export function useCreateMcpServer() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateMcpServerRequest) => createMcpServer(baseUrl as string, body),
    onSuccess: (data) => queryClient.setQueryData(keys(baseUrl).mcp, data),
  });
}

export function useMcp() {
  const { baseUrl } = useAuth();
  return useQuery<McpResponse>({
    queryKey: keys(baseUrl).mcp,
    queryFn: () => getMcp(baseUrl as string),
    enabled: baseUrl != null,
  });
}

export function useConfig() {
  const { baseUrl } = useAuth();
  return useQuery<ConfigResponse>({
    queryKey: keys(baseUrl).config,
    queryFn: () => getConfig(baseUrl as string),
    enabled: baseUrl != null,
  });
}

export function useUpdateConfig() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: UpdateConfigRequest) => updateConfig(baseUrl as string, body),
    onSuccess: (data) => {
      queryClient.setQueryData(keys(baseUrl).config, data);
    },
  });
}

export function useUsage(sessionId?: string | null) {
  const { baseUrl } = useAuth();
  const isFocused = useIsFocused();
  return useQuery<UsageResponse>({
    queryKey: ["usage", baseUrl, sessionId ?? null],
    queryFn: () => getUsage(baseUrl as string, sessionId ?? undefined),
    enabled: baseUrl != null,
    refetchInterval: isFocused ? 30000 : false,
    refetchOnWindowFocus: true,
  });
}

export function useSessionWeeklyDelta(sessionId: string | null) {
  const usage = useUsage(sessionId).data;
  const providers = usage?.session?.providers ?? [];
  const provider = providers
    .filter((item) => item.kind === "bridge" || item.kind === "oauth")
    .sort((a, b) => b.outputTokens - a.outputTokens)[0];
  const current = provider
    ? usage?.quota.find((item) => item.provider === provider.provider && item.windowKind === "weekly" && item.fraction != null)?.fraction
    : undefined;
  const key = provider && sessionId ? `${sessionId}:${provider.provider}` : null;
  useEffect(() => {
    if (key && current != null && !baselines.has(key)) baselines.set(key, current);
  }, [key, current]);
  if (!provider || current == null || !key) return { mode: "usd" as const };
  const baseline = baselines.get(key) ?? current;
  return { mode: "subscription" as const, provider: provider.provider, deltaPct: Math.max(0, current - baseline) * 100, weeklyFraction: current };
}
export function useForkSession() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, body }: { id: string; body: ForkSessionRequest }) => forkSession(baseUrl as string, id, body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).sessions });
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).sessionTree });
    },
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

export function useTranscribe() {
  const { baseUrl } = useAuth();
  return useMutation({
    mutationFn: ({ form, language }: { form: FormData; language?: string }) =>
      transcribeAudio(baseUrl as string, form, language),
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
  const activitySessionIdRef = useRef<string | null>(null);
  const activeRef = useRef(false);
  const [completed, setCompleted] = useState(false);

  useEffect(
    () => () => {
      const id = activityIdRef.current;
      const activeSessionId = activitySessionIdRef.current;
      activityIdRef.current = null;
      activitySessionIdRef.current = null;
      activeRef.current = false;
      if (id) endLiveActivity(id).catch(() => {});
      if (activeSessionId && baseUrl) {
        unsubscribePush(baseUrl, { session_id: activeSessionId, push_token: "" }).catch(() => {});
      }
    },
    [baseUrl],
  );

  const busy = snapshot?.busy ?? false;
  const waiting = snapshot != null && (snapshot.permission_prompt != null || snapshot.question != null);
  const sessionId = snapshot?.session_id ?? null;
  const title = snapshot?.title ?? "";
  const costUsd = snapshot?.cost_usd ?? 0;
  const contextTokens = snapshot?.context_tokens ?? 0;
  const contextLimit = snapshot?.context_limit ?? 0;

  useEffect(() => {
    const subscription = addLiveActivityPushTokenListener(({ sessionId: tokenSessionId, pushToken }) => {
      if (!baseUrl || tokenSessionId !== activitySessionIdRef.current) return;
      subscribePush(baseUrl, {
        session_id: tokenSessionId,
        push_token: pushToken,
        environment: __DEV__ ? "sandbox" : "production",
      }).catch(() => {});
    });
    return () => subscription.remove();
  }, [baseUrl]);

  // Detect the busy true->false edge in an effect (never read the ref during render).
  useEffect(() => {
    const wasBusy = prevBusyRef.current;
    const didComplete = wasBusy === true && !busy;
    const didBegin = busy && (!activeRef.current || activitySessionIdRef.current !== sessionId);
    prevBusyRef.current = busy;
    setCompleted(didComplete);
    if (didComplete && sessionId) {
      queryClient.invalidateQueries({ queryKey: keys(baseUrl).history(sessionId) });
    }

    if (!sessionId) return;
    if (didBegin) {
      activeRef.current = true;
      activitySessionIdRef.current = sessionId;
      startLiveActivity(sessionId, title, busy, waiting, costUsd, contextTokens, contextLimit)
        .then(({ activityId }) => {
          if (!activeRef.current || activitySessionIdRef.current !== sessionId) {
            if (activityId) endLiveActivity(activityId).catch(() => {});
            return;
          }
          activityIdRef.current = activityId;
          if (activityId) {
            updateLiveActivity(activityId, busy, waiting, costUsd, contextTokens, contextLimit).catch(() => {});
          }
        })
        .catch((error) => {
          console.warn("Failed to start Live Activity", error);
          activeRef.current = false;
          activitySessionIdRef.current = null;
        });
    } else if (didComplete) {
      const id = activityIdRef.current;
      const activeSessionId = activitySessionIdRef.current;
      activityIdRef.current = null;
      activitySessionIdRef.current = null;
      activeRef.current = false;
      if (id) endLiveActivity(id).catch(() => {});
      if (activeSessionId && baseUrl) {
        unsubscribePush(baseUrl, { session_id: activeSessionId, push_token: "" }).catch(() => {});
      }
    } else if (activityIdRef.current) {
      updateLiveActivity(activityIdRef.current, busy, waiting, costUsd, contextTokens, contextLimit).catch(
        () => {},
      );
    }
  }, [busy, waiting, sessionId, title, costUsd, contextTokens, contextLimit, baseUrl, queryClient]);

  return completed;
}
