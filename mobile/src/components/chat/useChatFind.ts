// Registers the transcript as a Find target. Hits come from the rows already loaded plus the
// host's own text search (`GET /api/history?q=`), which reaches pages that were never paged in;
// stepping to a hit parks a jump that the transcript's useMessageJump resolves, swapping in the
// window around an unloaded row with a single request.
import { useEffect, useRef } from "react";

import { getHistory, type HistoryRow } from "../../lib/api";
import { findController, type FindResult } from "../../lib/find/findController";
import { mergeHits } from "../../lib/find/chatHits";
import { steppedIndex } from "../../lib/find/matching";
import { requestMessageJump } from "../../lib/messageJump";
import { displayMessageText } from "./MessageRow";

/** Most rows the host is asked for; a full answer is shown as "N+". */
const REMOTE_HIT_LIMIT = 200;

export interface ChatFindOptions {
  enabled: boolean;
  baseUrl: string | null;
  sessionId: string;
  /** Loaded rows, newest first. */
  rows: readonly HistoryRow[];
  /** The hit now active, or null when the search is cleared. */
  onActivate: (seq: number | null) => void;
}

export function useChatFind({ enabled, baseUrl, sessionId, rows, onActivate }: ChatFindOptions): void {
  const rowsRef = useRef(rows);
  useEffect(() => {
    rowsRef.current = rows;
  });

  useEffect(() => {
    if (!enabled) return;
    let hits: number[] = [];
    let index = -1;
    let capped = false;
    let latest = 0;

    const activate = (next: number): FindResult => {
      index = next;
      const seq = hits[next];
      if (seq !== undefined) {
        onActivate(seq);
        requestMessageJump(sessionId, seq);
      }
      return { count: hits.length, index, capped };
    };

    return findController.register({
      id: "chat",
      label: "transcript",
      async search(query, caseSensitive) {
        const mine = ++latest;
        let remote: HistoryRow[] = [];
        if (baseUrl) {
          try {
            remote = await getHistory(baseUrl, { session: sessionId, q: query, limit: REMOTE_HIT_LIMIT });
            capped = remote.length >= REMOTE_HIT_LIMIT;
          } catch {
            capped = false;
          }
        }
        if (mine !== latest) return { count: hits.length, index, capped };
        hits = mergeHits(rowsRef.current, remote, query, caseSensitive, displayMessageText);
        // Starts at the newest hit: it is the one nearest the conversation's live end.
        return activate(hits.length - 1);
      },
      step(direction) {
        return activate(steppedIndex(index, hits.length, direction));
      },
      clear() {
        latest += 1;
        hits = [];
        index = -1;
        capped = false;
        onActivate(null);
      },
    });
  }, [enabled, baseUrl, sessionId, onActivate]);
}
