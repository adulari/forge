// react-query hooks over apiPr.ts. The PR reads, PR Watch and Ship are bridged over Anywhere (Ship
// and the PR Watch write only when the host opted in, see prAccess.ts); create/comment/merge and
// GitHub Actions still need a direct daemon connection, so those are disabled without one.
import { keepPreviousData, useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commentOnPr,
  createPr,
  dispatchWorkflow,
  getDispatchInputs,
  getPrDetail,
  getPrDiff,
  getPrList,
  getPrQueue,
  getPrStatus,
  getPrWatch,
  getPrWatches,
  getRunDetail,
  getWorkflowRuns,
  getWorkflows,
  mergePr,
  postPrQueueAction,
  runAction,
  setPrWatch,
  type PrState,
} from "./apiPr";
import { shipSession } from "./apiShip";
import { useAuth } from "./auth";
import { shouldPoll } from "../components/pr/prModel";
import { supportsDirectDaemonEndpoints } from "./transport";
import { usePrReadBase, useShipBase } from "./prAccess";

export { usePrAvailable } from "./prAccess";

const CHECKS_POLL_MS = 15_000;
const RUNS_POLL_MS = 10_000;

function useDirectBase(): string | null {
  const { baseUrl } = useAuth();
  return baseUrl && supportsDirectDaemonEndpoints(baseUrl) ? baseUrl : null;
}

export function usePrStatus(sessionId: string | null) {
  const baseUrl = usePrReadBase();
  return useQuery({
    queryKey: ["pr", "status", baseUrl, sessionId],
    queryFn: () => getPrStatus(baseUrl as string, sessionId as string),
    enabled: baseUrl != null && sessionId != null,
    refetchOnWindowFocus: true,
    staleTime: 10_000,
    refetchInterval: (query) => (shouldPoll(query.state.data?.pr) ? CHECKS_POLL_MS : false),
  });
}

export function usePrList(sessionId: string | null, state: PrState) {
  const baseUrl = usePrReadBase();
  return useQuery({
    queryKey: ["pr", "list", baseUrl, sessionId, state],
    queryFn: () => getPrList(baseUrl as string, sessionId as string, state),
    enabled: baseUrl != null && sessionId != null,
    staleTime: 20_000,
    placeholderData: keepPreviousData,
  });
}

export function usePrDetail(sessionId: string | null, number: number | null) {
  const baseUrl = usePrReadBase();
  return useQuery({
    queryKey: ["pr", "detail", baseUrl, sessionId, number],
    queryFn: () => getPrDetail(baseUrl as string, sessionId as string, number as number),
    enabled: baseUrl != null && sessionId != null && number != null,
    refetchInterval: (query) => (shouldPoll(query.state.data?.pr) ? CHECKS_POLL_MS : false),
  });
}

export function usePrDiff(
  sessionId: string | null,
  number: number | null,
  commit: string | null,
  ignoreWhitespace: boolean,
  enabled: boolean,
) {
  const baseUrl = usePrReadBase();
  return useQuery({
    queryKey: ["pr", "diff", baseUrl, sessionId, number, commit, ignoreWhitespace],
    queryFn: () =>
      getPrDiff(baseUrl as string, { session: sessionId as string, number: number as number, commit, ignoreWhitespace }),
    enabled: enabled && baseUrl != null && sessionId != null && number != null,
    placeholderData: keepPreviousData,
  });
}

function usePrInvalidate() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: ["pr"] });
}

export function useCreatePr() {
  const baseUrl = useDirectBase();
  const invalidate = usePrInvalidate();
  return useMutation({
    mutationFn: (body: Parameters<typeof createPr>[1]) => createPr(baseUrl as string, body),
    onSuccess: invalidate,
  });
}

export function useCommentOnPr() {
  const baseUrl = useDirectBase();
  const invalidate = usePrInvalidate();
  return useMutation({
    mutationFn: (body: Parameters<typeof commentOnPr>[1]) => commentOnPr(baseUrl as string, body),
    onSuccess: invalidate,
  });
}

export function useMergePr() {
  const baseUrl = useDirectBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof mergePr>[1]) => mergePr(baseUrl as string, body),
    onSuccess: (_result, body) => {
      void queryClient.invalidateQueries({ queryKey: ["pr"] });
      void queryClient.invalidateQueries({ queryKey: ["git"] });
      void queryClient.invalidateQueries({ queryKey: ["actions"] });
      return body;
    },
  });
}

export function useWorkflows(sessionId: string | null) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["actions", "workflows", baseUrl, sessionId],
    queryFn: () => getWorkflows(baseUrl as string, sessionId as string),
    enabled: baseUrl != null && sessionId != null,
    staleTime: 60_000,
  });
}

export function useWorkflowRuns(sessionId: string | null, workflow?: string) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["actions", "runs", baseUrl, sessionId, workflow ?? null],
    queryFn: () => getWorkflowRuns(baseUrl as string, sessionId as string, workflow),
    enabled: baseUrl != null && sessionId != null,
    refetchOnWindowFocus: true,
    refetchInterval: (query) =>
      query.state.data?.items.some((run) => run.status !== "completed") ? RUNS_POLL_MS : false,
  });
}

export function useRunDetail(sessionId: string | null, id: number | null) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["actions", "run", baseUrl, sessionId, id],
    queryFn: () => getRunDetail(baseUrl as string, sessionId as string, id as number),
    enabled: baseUrl != null && sessionId != null && id != null,
    refetchInterval: (query) => (query.state.data?.status !== "completed" ? RUNS_POLL_MS : false),
  });
}

export function useDispatchInputs(sessionId: string | null, workflow: string | null) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["actions", "inputs", baseUrl, sessionId, workflow],
    queryFn: () => getDispatchInputs(baseUrl as string, sessionId as string, workflow as string),
    enabled: baseUrl != null && sessionId != null && workflow != null,
    staleTime: 5 * 60_000,
  });
}

export function useDispatchWorkflow() {
  const baseUrl = useDirectBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof dispatchWorkflow>[1]) => dispatchWorkflow(baseUrl as string, body),
    onSuccess: () => {
      // GitHub registers the run a moment after the dispatch returns.
      setTimeout(() => void queryClient.invalidateQueries({ queryKey: ["actions", "runs"] }), 2_500);
    },
  });
}

export function useRunAction(verb: "rerun" | "cancel" | "delete") {
  const baseUrl = useDirectBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof runAction>[2]) => runAction(baseUrl as string, verb, body),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["actions"] }),
  });
}

/** Open pull requests across every project the host knows, one `gh` call per distinct directory. */
export function useAllPrs(sessionIds: readonly string[], state: PrState) {
  const baseUrl = usePrReadBase();
  return useQueries({
    queries: sessionIds.map((sessionId) => ({
      queryKey: ["pr", "list", baseUrl, sessionId, state],
      queryFn: () => getPrList(baseUrl as string, sessionId, state),
      enabled: baseUrl != null,
      staleTime: 20_000,
    })),
  });
}

const WATCH_POLL_MS = 10_000;

export function usePrWatch(sessionId: string | null) {
  const baseUrl = usePrReadBase();
  return useQuery({
    queryKey: ["pr", "watch", baseUrl, sessionId],
    queryFn: () => getPrWatch(baseUrl as string, sessionId as string),
    enabled: baseUrl != null && sessionId != null,
    refetchInterval: (query) => (query.state.data?.watch?.state === "watching" ? WATCH_POLL_MS : false),
  });
}

export function usePrWatches() {
  const baseUrl = usePrReadBase();
  return useQuery({
    queryKey: ["pr", "watches", baseUrl],
    queryFn: () => getPrWatches(baseUrl as string),
    enabled: baseUrl != null,
    refetchInterval: WATCH_POLL_MS,
  });
}

export function useSetPrWatch() {
  const baseUrl = useShipBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof setPrWatch>[1]) => setPrWatch(baseUrl as string, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["pr", "watch"] });
      void queryClient.invalidateQueries({ queryKey: ["pr", "watches"] });
    },
  });
}

export function useShipSession() {
  const baseUrl = useShipBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: { sessionId: string; request: Parameters<typeof shipSession>[2] }) =>
      shipSession(baseUrl as string, args.sessionId, args.request),
    // A refusal can still have committed, so the working tree and branch lists refresh either way.
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["pr"] });
      void queryClient.invalidateQueries({ queryKey: ["git"] });
    },
  });
}

const QUEUE_POLL_MS = 5_000;

/** Merge queue standing per repo. Bridged, so any signed-in connection works; polls while mounted. */
export function usePrQueue() {
  const { baseUrl } = useAuth();
  return useQuery({
    queryKey: ["pr", "queue", baseUrl],
    queryFn: () => getPrQueue(baseUrl as string).then((r) => r.queues),
    enabled: baseUrl != null,
    refetchInterval: QUEUE_POLL_MS,
  });
}

export function useQueueAction() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof postPrQueueAction>[1]) => postPrQueueAction(baseUrl as string, body),
    onSuccess: (result) => {
      // The reply already carries the fresh queues; show them before the refetch lands.
      queryClient.setQueryData(["pr", "queue", baseUrl], result.queues);
      void queryClient.invalidateQueries({ queryKey: ["pr", "queue"] });
      void queryClient.invalidateQueries({ queryKey: ["pr", "watch"] });
      void queryClient.invalidateQueries({ queryKey: ["pr", "watches"] });
    },
  });
}
