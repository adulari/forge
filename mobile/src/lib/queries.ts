// react-query v5 hooks over api.ts. Screens call ONLY these hooks (UI_RULES.md #3) —
// never raw fetch. Query keys are namespaced by baseUrl so switching a paired server
// never serves stale cross-server data from the persisted cache.
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useIsFocused } from "@react-navigation/native";

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
  uploadFile,
} from "./api";
import { useAuth } from "./auth";

const SESSIONS_POLL_MS = 3000;
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
  return useQuery<SessionRow[]>({
    queryKey: keys(baseUrl).sessions,
    queryFn: () => getSessions(baseUrl as string),
    enabled: baseUrl != null,
    refetchInterval: isFocused ? SESSIONS_POLL_MS : false,
    refetchOnWindowFocus: true,
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
