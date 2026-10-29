import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { getHistory, type HistoryRow } from "../../lib/api";
import { bucketTicks, isRailPrompt, mergePrompts, railTicks, type RailTick } from "../../lib/promptRail";
import { displayMessageText } from "./MessageRow";

/** Most prompts the outline asks the host for. */
const OUTLINE_LIMIT = 200;

/** Every user prompt of the session as rail ticks: the host's outline (so prompts that were never
 * paged in appear) merged with the loaded rows (so a prompt just sent does too). */
export function usePromptRail(
  baseUrl: string | null,
  sessionId: string,
  loaded: readonly HistoryRow[],
  enabled: boolean,
): { ticks: RailTick[]; total: number } {
  const newest = loaded.find(isRailPrompt)?.seq ?? -1;
  const outline = useQuery({
    queryKey: ["promptOutline", baseUrl, sessionId, newest],
    queryFn: () => getHistory(baseUrl as string, { session: sessionId, role: "user", limit: OUTLINE_LIMIT }),
    enabled: enabled && baseUrl != null,
    staleTime: 30_000,
    retry: false,
    placeholderData: keepPreviousData,
  });
  const remote = outline.data;
  return useMemo(() => {
    if (!enabled) return { ticks: [], total: 0 };
    const all = railTicks(mergePrompts(remote ?? [], loaded, displayMessageText));
    return { ticks: bucketTicks(all), total: all.length };
  }, [enabled, remote, loaded]);
}
