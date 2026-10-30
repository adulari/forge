import { useMutation, useQueryClient } from "@tanstack/react-query";

import { request, type CreateMcpServerRequest, type McpResponse, type UpdateMcpServerRequest } from "./api";
import { useAuth } from "./auth";
import type { McpProbe } from "./mcpHelpers";

export { joinArgs, mcpStatusMeta, mcpToolSummary, splitArgs, toggleToolDisabled } from "./mcpHelpers";
export type { McpProbe, McpStatusTone } from "./mcpHelpers";

export type EditMcpServerRequest = CreateMcpServerRequest;

const mcpKey = (baseUrl: string | null) => ["mcp", baseUrl] as const;

function json(method: string, body: unknown): RequestInit {
  return { method, body: JSON.stringify(body) };
}

function useMcpMutation<V, R>(fn: (baseUrl: string, value: V) => Promise<R>, seed: (data: R) => McpResponse | null) {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (value: V) => fn(baseUrl as string, value),
    onSuccess: (data) => {
      const page = seed(data);
      if (page) queryClient.setQueryData(mcpKey(baseUrl), page);
    },
  });
}

const page = (data: McpResponse) => data;

export function useReloadMcp() {
  return useMcpMutation((baseUrl: string, _: void) => request<McpResponse>(baseUrl, "/api/mcp/reload", json("POST", {}), 120_000), page);
}

export function useProbeMcp() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => request<McpProbe>(baseUrl as string, "/api/mcp/probe", json("POST", { name }), 120_000),
    onSettled: () => void queryClient.invalidateQueries({ queryKey: mcpKey(baseUrl) }),
  });
}

export function useEditMcpServer() {
  return useMcpMutation((baseUrl: string, body: EditMcpServerRequest) => request<McpResponse>(baseUrl, "/api/mcp", json("PUT", body)), page);
}

export function useDeleteMcpServer() {
  return useMcpMutation((baseUrl: string, name: string) => request<McpResponse>(baseUrl, "/api/mcp", json("DELETE", { name })), page);
}

export function useSetDisabledTools() {
  return useMcpMutation(
    (baseUrl: string, body: Required<Pick<UpdateMcpServerRequest, "name" | "disabled_tools">>) => request<McpResponse>(baseUrl, "/api/mcp", json("PATCH", body)),
    page,
  );
}

export function useUpdateMcpBuiltin() {
  return useMcpMutation(
    (baseUrl: string, body: { id: string; enabled: boolean }) => request<McpResponse>(baseUrl, "/api/mcp/builtin", json("PATCH", body)),
    page,
  );
}
