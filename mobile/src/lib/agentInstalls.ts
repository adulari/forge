// Install and update the subscription CLIs (claude, codex, agy) on the daemon's host. The daemon
// runs each vendor's own installer or updater; this file is the client side of that job API.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { request } from "./api";
import { useAuth } from "./auth";
import { anyJobActive, type AgentInstallsResponse } from "./agentInstallsCore";
import { supportsDirectDaemonEndpoints } from "./transport";

export * from "./agentInstallsCore";

export function getAgentInstalls(baseUrl: string, refresh = false): Promise<AgentInstallsResponse> {
  return request(baseUrl, refresh ? "/api/agent-installs?refresh=1" : "/api/agent-installs");
}

export function runAgentInstall(baseUrl: string, id: string): Promise<{ started: string[] }> {
  return request(baseUrl, `/api/agent-installs/${encodeURIComponent(id)}/run`, { method: "POST" });
}

export function updateAllAgents(baseUrl: string): Promise<{ started: string[] }> {
  return request(baseUrl, "/api/agent-installs/update-all", { method: "POST" });
}

export function useAgentInstalls() {
  const { baseUrl } = useAuth();
  return useQuery<AgentInstallsResponse>({
    queryKey: ["agentInstalls", baseUrl],
    queryFn: () => getAgentInstalls(baseUrl as string),
    enabled: baseUrl != null && supportsDirectDaemonEndpoints(baseUrl),
    staleTime: 60_000,
    retry: false,
    refetchInterval: (query) => (anyJobActive(query.state.data?.agents ?? []) ? 1200 : false),
  });
}

export function useRunAgentInstall() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => runAgentInstall(baseUrl as string, id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["agentInstalls", baseUrl] }),
  });
}

export function useUpdateAllAgents() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => updateAllAgents(baseUrl as string),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["agentInstalls", baseUrl] }),
  });
}

export function useRefreshAgentInstalls() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return () =>
    getAgentInstalls(baseUrl as string, true).then((data) => {
      queryClient.setQueryData(["agentInstalls", baseUrl], data);
    });
}
