import { useQuery } from "@tanstack/react-query";

import { getUsageActivity, type UsageActivityResponse } from "./api";
import { useAuth } from "./auth";

export function useUsageActivity() {
  const { baseUrl } = useAuth();
  return useQuery<UsageActivityResponse>({
    queryKey: ["usage-activity", baseUrl],
    queryFn: () => getUsageActivity(baseUrl as string),
    enabled: baseUrl != null,
    staleTime: 60_000,
  });
}
