// What the warm-start cache keeps on disk. The whole react-query cache was persisted as one
// AsyncStorage value, re-serialized at most once a second on every change and parsed in full
// before any screen could query anything. Measured on a fresh desktop instance it was 587 KB
// after opening one session, 95% of it two changelog responses; every opened session added all
// of its loaded history pages (tool results included, up to ~60 KB a page) for 24 hours. While a
// turn streams, history refreshes every few seconds, so that blob was stringified over and over
// on the JS thread the reply renders on, and a cold start parsed all of it before showing
// anything. Only what makes a cold open look instant is kept: the first history page of the most
// recently updated sessions, and the small lists. Everything else refetches.
import type { PersistedClient } from "@tanstack/react-query-persist-client";

/** Sessions whose newest history page survives a restart. */
export const PERSISTED_HISTORY_SESSIONS = 6;

/** Query families never written to disk: large and cheap to refetch. */
const NOT_PERSISTED = new Set(["changelog", "git", "workspace", "diagnostics"]);

interface InfiniteLike {
  pages: unknown[];
  pageParams: unknown[];
}

function isInfinite(data: unknown): data is InfiniteLike {
  return (
    data != null &&
    typeof data === "object" &&
    Array.isArray((data as InfiniteLike).pages) &&
    Array.isArray((data as InfiniteLike).pageParams)
  );
}

export function trimPersistedClient(client: PersistedClient): PersistedClient {
  const queries = client.clientState.queries.filter((q) => !NOT_PERSISTED.has(String(q.queryKey[0])));
  const history = queries
    .filter((q) => q.queryKey[0] === "history")
    .sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt);
  const keep = new Set(history.slice(0, PERSISTED_HISTORY_SESSIONS).map((q) => q.queryHash));
  const trimmed = queries
    .filter((q) => q.queryKey[0] !== "history" || keep.has(q.queryHash))
    .map((q) => {
      const data = q.state.data;
      if (q.queryKey[0] !== "history" || !isInfinite(data) || data.pages.length <= 1) return q;
      return { ...q, state: { ...q.state, data: { pages: data.pages.slice(0, 1), pageParams: data.pageParams.slice(0, 1) } } };
    });
  return { ...client, clientState: { ...client.clientState, queries: trimmed } };
}

export function serializePersistedClient(client: PersistedClient): string {
  return JSON.stringify(trimPersistedClient(client));
}
