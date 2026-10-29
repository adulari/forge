// The rows the transcript shows. Normally that is the history query's pages, newest first. A jump
// to a row that is not loaded swaps in a window fetched around it with one request
// (`GET /api/history?around=<seq>`) instead of paging through everything in between, and the
// transcript stays on that window ("detached" from the live end) until the person returns to the
// latest or sends a message.
import type { UseInfiniteQueryResult, InfiniteData } from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";

import { getHistory, type HistoryRow } from "./api";
import { historyPageSize } from "./historyPaging";
import { windowHolds } from "./historyWindow";

interface Detached {
  sessionId: string;
  pages: HistoryRow[][];
  hasMore: boolean;
}

export interface HistoryView {
  rows: HistoryRow[];
  detached: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  isError: boolean;
  fetchNextPage: () => unknown;
  /** Loads the window around `seq` and shows it; false when the host cannot (older build). */
  jumpAround: (seq: number) => Promise<boolean>;
  returnToLatest: () => void;
}

export function useHistoryView(
  baseUrl: string | null,
  sessionId: string,
  query: UseInfiniteQueryResult<InfiniteData<HistoryRow[], unknown>, Error>,
  liveRows: HistoryRow[],
): HistoryView {
  const [detached, setDetached] = useState<Detached | null>(null);
  const [fetching, setFetching] = useState(false);
  const live = detached != null && detached.sessionId === sessionId ? detached : null;
  const limit = historyPageSize(true);

  const detachedRows = useMemo(() => (live ? live.pages.flat() : null), [live]);

  const jumpAround = useCallback(
    async (seq: number) => {
      if (baseUrl == null) return false;
      try {
        const rows = await getHistory(baseUrl, { session: sessionId, limit, around: seq, include_tools: true });
        if (!windowHolds(rows, seq)) return false;
        setDetached({ sessionId, pages: [rows], hasMore: rows.length >= limit });
        return true;
      } catch {
        return false;
      }
    },
    [baseUrl, sessionId, limit],
  );

  const fetchOlderDetached = useCallback(async () => {
    if (!live || baseUrl == null || fetching || !live.hasMore) return;
    const oldest = live.pages.flat().at(-1)?.seq;
    if (oldest === undefined) return;
    setFetching(true);
    try {
      const page = await getHistory(baseUrl, { session: sessionId, limit, before: oldest, include_tools: true });
      setDetached((current) =>
        current && current.sessionId === sessionId
          ? { ...current, pages: [...current.pages, page], hasMore: page.length >= limit }
          : current,
      );
    } catch {
      setDetached((current) => (current ? { ...current, hasMore: false } : current));
    } finally {
      setFetching(false);
    }
  }, [live, baseUrl, sessionId, limit, fetching]);

  const returnToLatest = useCallback(() => setDetached(null), []);

  if (live && detachedRows) {
    return {
      rows: detachedRows,
      detached: true,
      hasNextPage: live.hasMore,
      isFetchingNextPage: fetching,
      isError: false,
      fetchNextPage: fetchOlderDetached,
      jumpAround,
      returnToLatest,
    };
  }
  return {
    rows: liveRows,
    detached: false,
    hasNextPage: query.hasNextPage,
    isFetchingNextPage: query.isFetchingNextPage,
    isError: query.isError,
    fetchNextPage: query.fetchNextPage,
    jumpAround,
    returnToLatest,
  };
}
