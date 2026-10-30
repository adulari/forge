// react-query hooks over apiAccess.ts. Every query needs a direct daemon connection.
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getRemoteAccess, getWhoAmI, pairDevice, revokeClient, type PairRequest } from "./apiAccess";
import { useAuth } from "./auth";
import { accessFrom, FULL_ACCESS, type ConnectionAccess } from "./remoteAccessModel";
import { supportsDirectDaemonEndpoints } from "./transport";

function useDirectBase(): string | null {
  const { baseUrl } = useAuth();
  return baseUrl && supportsDirectDaemonEndpoints(baseUrl) ? baseUrl : null;
}

export function useDirectAccessAvailable(): boolean {
  return useDirectBase() != null;
}

/** What this connection may do. Full access until the host answers, and for hosts without scopes. */
export function useConnectionAccess(): ConnectionAccess {
  const baseUrl = useDirectBase();
  const query = useQuery({
    queryKey: ["access", "whoami", baseUrl],
    queryFn: async () => {
      try {
        return await getWhoAmI(baseUrl as string);
      } catch {
        return null;
      }
    },
    enabled: baseUrl != null,
    staleTime: 60_000,
  });
  return query.data ? accessFrom(query.data) : FULL_ACCESS;
}

export function useRemoteAccess(enabled = true) {
  const baseUrl = useDirectBase();
  return useQuery({
    queryKey: ["access", "remote", baseUrl],
    queryFn: () => getRemoteAccess(baseUrl as string),
    enabled: enabled && baseUrl != null,
    refetchInterval: 10_000,
    staleTime: 5_000,
  });
}

export function usePairDevice() {
  const baseUrl = useDirectBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: PairRequest) => pairDevice(baseUrl as string, body),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["access", "remote"] }),
  });
}

export function useRevokeClient() {
  const baseUrl = useDirectBase();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => revokeClient(baseUrl as string, id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["access", "remote"] }),
  });
}
