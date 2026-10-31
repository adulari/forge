// react-query hooks over apiControl.ts. Every query needs a direct daemon connection.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getControl, setControlTool, type ControlOverview } from "./apiControl";
import { useAuth } from "./auth";
import { supportsDirectDaemonEndpoints } from "./transport";

export function useControlAvailable(): boolean {
  const { baseUrl } = useAuth();
  return baseUrl != null && supportsDirectDaemonEndpoints(baseUrl);
}

export function useControl() {
  const { baseUrl } = useAuth();
  const direct = baseUrl != null && supportsDirectDaemonEndpoints(baseUrl);
  return useQuery<ControlOverview>({
    queryKey: ["control", baseUrl],
    queryFn: () => getControl(baseUrl as string),
    enabled: direct,
    refetchInterval: 10_000,
    staleTime: 5_000,
  });
}

export function useSetControlTool() {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) =>
      setControlTool(baseUrl as string, name, enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["control", baseUrl] }),
  });
}
