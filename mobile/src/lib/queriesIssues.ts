// react-query hooks over apiIssues.ts. Bridged routes, so any signed-in connection works.
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";

import { getIssueDetail, getIssueList, startIssue, type IssueState } from "./apiIssues";
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
