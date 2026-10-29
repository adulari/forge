import { matchOffsets } from "../../lib/find/matching";
import { type AnsiLine, lineText } from "./ansi";

export interface TerminalHit {
  /** `AnsiLine.key`: stable while the line is retained, so a hit survives output arriving below it. */
  key: number;
  start: number;
  end: number;
}

export interface LineHighlight {
  start: number;
  end: number;
  active: boolean;
}

export interface TextPiece {
  text: string;
  /** `null` outside any match. */
  active: boolean | null;
}

export const MAX_TERMINAL_HITS = 2000;

/** Every match in the retained scrollback, oldest first. A match never spans two lines. */
export function findTerminalHits(
  lines: readonly AnsiLine[],
  query: string,
  caseSensitive: boolean,
  cap = MAX_TERMINAL_HITS,
): { hits: TerminalHit[]; capped: boolean } {
  const hits: TerminalHit[] = [];
  if (query === "") return { hits, capped: false };
  for (const line of lines) {
    for (const start of matchOffsets(lineText(line), query, caseSensitive, cap - hits.length)) {
      hits.push({ key: line.key, start, end: start + query.length });
    }
    if (hits.length >= cap) return { hits, capped: true };
  }
  return { hits, capped: false };
}

/** Where the hit that was active before a refresh sits in the new list: the same spot when the
 * line is still retained, else the first hit after it, else the last. */
export function relocateHit(hits: readonly TerminalHit[], previous: TerminalHit | undefined): number {
  if (hits.length === 0) return -1;
  if (!previous) return hits.length - 1;
  const same = hits.findIndex((hit) => hit.key === previous.key && hit.start === previous.start);
  if (same >= 0) return same;
  const after = hits.findIndex((hit) => hit.key > previous.key);
  return after >= 0 ? after : hits.length - 1;
}

export function highlightsByLine(hits: readonly TerminalHit[], active: number): Map<number, LineHighlight[]> {
  const out = new Map<number, LineHighlight[]>();
  hits.forEach((hit, index) => {
    const list = out.get(hit.key) ?? [];
    list.push({ start: hit.start, end: hit.end, active: index === active });
    out.set(hit.key, list);
  });
  return out;
}

/** Splits `text`, which begins `offset` characters into its line, at the highlight edges. */
export function splitByHighlights(text: string, offset: number, highlights: readonly LineHighlight[] | undefined): TextPiece[] {
  if (!highlights || highlights.length === 0 || text === "") return [{ text, active: null }];
  const end = offset + text.length;
  const pieces: TextPiece[] = [];
  let at = offset;
  for (const highlight of highlights) {
    const from = Math.max(highlight.start, at);
    const to = Math.min(highlight.end, end);
    if (to <= from) continue;
    if (from > at) pieces.push({ text: text.slice(at - offset, from - offset), active: null });
    pieces.push({ text: text.slice(from - offset, to - offset), active: highlight.active });
    at = to;
  }
  if (at < end) pieces.push({ text: text.slice(at - offset), active: null });
  return pieces;
}
