// react-query hooks over apiProjectActions.ts. Like the PR routes these need a direct daemon
// connection, so every query is disabled without one. Polling pauses while the page is hidden
// (react-query's default), which is the "only while visible" rule.
import { useQuery } from "@tanstack/react-query";

import { getProjectActions, getRunningActions, getSessionPorts } from "./apiProjectActions";
import { useAuth } from "./auth";
import { supportsDirectDaemonEndpoints } from "./transport";

const ACTIONS_POLL_MS = 3_000;
const PORTS_POLL_MS = 3_000;
const RUNNING_POLL_MS = 5_000;

export function useDirectBase(): string | null {
  const { baseUrl } = useAuth();
  return baseUrl && supportsDirectDaemonEndpoints(baseUrl) ? baseUrl : null;
}

export function useProjectActions(sessionId: string | null, enabled = true, poll = true) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["project-actions", baseUrl, sessionId],
    queryFn: () => getProjectActions(baseUrl as string, sessionId as string),
    enabled: enabled && baseUrl != null && sessionId != null,
    staleTime: 1_500,
    refetchInterval: poll ? ACTIONS_POLL_MS : false,
  });
}

export function useSessionPorts(sessionId: string | null, enabled = true) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["session-ports", baseUrl, sessionId],
    queryFn: () => getSessionPorts(baseUrl as string, sessionId as string),
    enabled: enabled && baseUrl != null && sessionId != null,
    staleTime: 1_500,
    refetchInterval: PORTS_POLL_MS,
  });
}

export function useRunningActions(enabled = true) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["project-actions-running", baseUrl],
    queryFn: () => getRunningActions(baseUrl as string),
    enabled: enabled && baseUrl != null,
    staleTime: 2_500,
    refetchInterval: RUNNING_POLL_MS,
    retry: false,
  });
}
