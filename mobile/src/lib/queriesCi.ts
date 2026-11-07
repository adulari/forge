import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getSessionCi, runSessionCi, type CiPlanResponse } from "./apiCi";
import { useAuth } from "./auth";

const ciKey = (baseUrl: string | null, sessionId: string | null) => ["ci", baseUrl, sessionId] as const;
const POLL_MS = 3_000;

/** The session's local CI plan with its last run. Only fetched while the sheet is open; polls
 * while a run is going so a run that outlives the request still lands. */
export function useSessionCi(sessionId: string | null, enabled: boolean, poll: boolean) {
  const { baseUrl } = useAuth();
  return useQuery<CiPlanResponse>({
    queryKey: ciKey(baseUrl, sessionId),
    queryFn: () => getSessionCi(baseUrl as string, sessionId as string),
    enabled: baseUrl != null && sessionId != null && enabled,
    staleTime: 0,
    retry: false,
    refetchInterval: poll ? POLL_MS : false,
  });
}

/** Runs the plan on the host. The answer is the plan with the new results, so it replaces the
 * cached plan directly. */
export function useRunSessionCi(sessionId: string | null) {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => runSessionCi(baseUrl as string, sessionId as string),
    onSuccess: (plan) => queryClient.setQueryData(ciKey(baseUrl, sessionId), plan),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: ciKey(baseUrl, sessionId) }),
  });
}
