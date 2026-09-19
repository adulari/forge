import { useCallback, useState } from "react";

/**
 * Pull-to-refresh state that only reflects the user's own pull. React Query's `isFetching` /
 * `isRefetching` are also true for every interval poll and focus refetch, so wiring them to a
 * RefreshControl parked the native spinner over the list on each poll — over the relay, most of
 * the time.
 */
export function usePullRefresh(refetch: () => Promise<unknown>): {
  refreshing: boolean;
  onRefresh: () => void;
} {
  const [refreshing, setRefreshing] = useState(false);
  const onRefresh = useCallback(() => {
    setRefreshing(true);
    void refetch().finally(() => setRefreshing(false));
  }, [refetch]);
  return { refreshing, onRefresh };
}
