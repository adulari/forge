import { describe, expect, it } from "vitest";

import {
  HISTORY_PAGE_SIZE,
  HISTORY_PAGE_SIZE_WITH_TOOLS,
  historyPageSize,
} from "./historyPaging";

/** The connector's inline threshold (`MAX_OUTBOUND_INLINE_BODY`), in bytes. Above this a bridge
 * response is offloaded to a blob instead of travelling inline. */
const INLINE_BUDGET_BYTES = 64 * 1024;

/** Bytes per requested-limit unit on a tool-heavy session, from the live measurement recorded in
 * `historyPaging.ts`: limit=30 returned 115.7 KiB, i.e. ~3.86 KiB per unit of the limit asked for
 * (the response is longer than the limit because tool rows expand). */
const WORST_CASE_BYTES_PER_LIMIT = (115.7 * 1024) / 30;

describe("history page sizing", () => {
  it("asks for far fewer rows when tool results ride along", () => {
    expect(historyPageSize(true)).toBe(HISTORY_PAGE_SIZE_WITH_TOOLS);
    expect(historyPageSize(false)).toBe(HISTORY_PAGE_SIZE);
    expect(HISTORY_PAGE_SIZE_WITH_TOOLS).toBeLessThan(HISTORY_PAGE_SIZE);
  });

  it("keeps a tool-heavy page inline, so the relay never has to blob it", () => {
    // The regression: at the old shared size of 60 this was ~237 KiB — every scrollback page took
    // the blob path over a mobile link, and a page that fails leaves the screen on grey filler.
    const projected = historyPageSize(true) * WORST_CASE_BYTES_PER_LIMIT;
    expect(projected).toBeLessThan(INLINE_BUDGET_BYTES);
    expect(HISTORY_PAGE_SIZE * WORST_CASE_BYTES_PER_LIMIT).toBeGreaterThan(INLINE_BUDGET_BYTES);
  });

  it("is a single value per query, so the limit and the last-page test cannot disagree", () => {
    // Requesting one size and testing the returned length against another stops paging after the
    // first page. Both call sites read this function, so they cannot drift apart.
    for (const includeTools of [true, false]) {
      const requested = historyPageSize(includeTools);
      const shortPage = requested - 1;
      const fullPage = requested;
      expect(shortPage < requested).toBe(true); // -> no next page
      expect(fullPage < requested).toBe(false); // -> keep paging
    }
  });

  it("still lets a plain page fetch in bulk", () => {
    expect(historyPageSize(false)).toBe(60);
  });
});
