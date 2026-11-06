// react-query hooks over apiIssues.ts. Bridged routes, so any signed-in connection works.
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  cancelIssueDrain,
  getIssueDetail,
  getIssueDrains,
  getIssueList,
  startIssue,
  startIssueDrain,
  type IssueDrain,
  type IssueState,
} from "./apiIssues";
import { useAuth } from "./auth";

/** Open issues across every project the host knows, one `gh` call per distinct directory. */
export function useAllIssues(sessionIds: readonly string[], state: IssueState) {
  const { baseUrl } = useAuth();
  return useQueries({
    queries: sessionIds.map((sessionId) => ({
      queryKey: ["issues", "list", baseUrl, sessionId, state],
      queryFn: () => getIssueList(baseUrl as string, sessionId, state),
      enabled: baseUrl != null,
      staleTime: 30_000,
    })),
  });
}

export function useIssueDetail(sessionId: string | null, number: number | null) {
  const { baseUrl } = useAuth();
  return useQuery({
    queryKey: ["issues", "detail", baseUrl, sessionId, number],
    queryFn: () => getIssueDetail(baseUrl as string, sessionId as string, number as number),
    enabled: baseUrl != null && sessionId != null && number != null,
  });
}

export function useStartIssue() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof startIssue>[1]) => startIssue(baseUrl as string, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      void queryClient.invalidateQueries({ queryKey: ["pr", "watches"] });
    },
  });
}

/** Every drain on the host, newest first. Polls while one is still going. */
export function useIssueDrains() {
  const { baseUrl } = useAuth();
  return useQuery({
    queryKey: ["issues", "drains", baseUrl],
    queryFn: () => getIssueDrains(baseUrl as string).then((r) => r.drains),
    enabled: baseUrl != null,
    refetchInterval: (query) => (query.state.data?.some(isLiveDrain) ? 5_000 : false),
  });
}

export const isLiveDrain = (drain: IssueDrain): boolean => drain.status === "active" || drain.status === "paused";

export function useStartDrain() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: Parameters<typeof startIssueDrain>[1]) => startIssueDrain(baseUrl as string, body),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["issues", "drains"] });
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });
}

export function useCancelDrain() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => cancelIssueDrain(baseUrl as string, id),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ["issues", "drains"] }),
  });
}
