// How large a transcript history page may be, which is a transport question, not a UI one.
//
// On loopback any page size works. Over the Anywhere relay it does not: the connector sends a
// bridge response inline up to `MAX_OUTBOUND_INLINE_BODY` (64 KiB) and offloads anything larger to
// a blob, and bodies do not travel as bytes — they are `Vec<u8>` inside JSON, ~3.57x once encoded —
// against a relay envelope ceiling of ~300 KiB.
//
// A TOOLS page is a different size class from a plain one: it carries whole tool results, and one
// assistant carrier expands into a row per call it made, so it is both wider per row and longer
// than the limit asked for. Measured against a real tool-heavy session:
//
//   limit=30 include_tools=1 -> 49 rows, 115.7 KiB     limit=30 (no tools) ->  9.6 KiB
//   limit=50 include_tools=1 -> 86 rows, 227.4 KiB     limit=50 (no tools) -> 16.5 KiB
//
// At the old shared size of 60 the chat's tools page was hundreds of KiB before encoding, forcing
// the blob path on every scrollback page for a phone on a mobile link. When a page does not
// arrive, the session screen stays on the grey transcript filler: history looks greyed out, it
// will not scroll back, and tool rows are inert because filler rows carry synthetic seqs that
// disable every seq-keyed action.

/** Plain (no tool rows) page size — small rows, safe to fetch in bulk. */
export const HISTORY_PAGE_SIZE = 60;

/** Tools page size. Keeps even a pathological session's page inline (~58 KiB by the measurement
 * above) and an ordinary one far below it. The cost is more round trips when scrolling far back,
 * which is the cheaper failure of the two. */
export const HISTORY_PAGE_SIZE_WITH_TOOLS = 15;

/** The page size for a history query.
 *
 * The caller MUST use this one value for both the request limit and the "was this the last page?"
 * comparison. Requesting 15 and then testing the returned length against 60 reads as "short page,
 * stop paging" and kills scrollback after the first page — the same broken-looking screen this
 * sizing exists to prevent. */
export function historyPageSize(includeTools: boolean): number {
  return includeTools ? HISTORY_PAGE_SIZE_WITH_TOOLS : HISTORY_PAGE_SIZE;
}
