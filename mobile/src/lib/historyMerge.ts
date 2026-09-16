// Folding a freshly fetched newest history page into an already-loaded infinite query.
import type { InfiniteData } from "@tanstack/react-query";

import type { HistoryRow } from "./api";

/** `data` with its first page replaced by `fresh` (newest-first), keeping every older page.
 *
 * Rows older than the fresh page survive from the old first page, so nothing already on screen
 * disappears. When the fresh page does not reach back to the newest row already loaded, the rows
 * in between are unknown, so the loaded pages are dropped and the query restarts from `fresh`
 * rather than showing a timeline with a silent hole in it. A tool carrier's call rows share its
 * seq and a page never splits a carrier, so filtering by seq cannot drop half of one. */
export function mergeNewestHistoryPage(
  data: InfiniteData<HistoryRow[], unknown> | undefined,
  fresh: HistoryRow[],
): InfiniteData<HistoryRow[], unknown> | undefined {
  if (!data || data.pages.length === 0 || fresh.length === 0) return data;
  const oldestFresh = fresh[fresh.length - 1].seq;
  const [first, ...rest] = data.pages;
  const newestKnown = first[0]?.seq;
  if (newestKnown === undefined || newestKnown < oldestFresh) {
    return { pages: [fresh], pageParams: data.pageParams.slice(0, 1) };
  }
  return {
    pages: [[...fresh, ...first.filter((row) => row.seq < oldestFresh)], ...rest],
    pageParams: data.pageParams,
  };
}
