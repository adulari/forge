/** Most hits one search reports; a longer list is cut and flagged so the bar can say "N+". */
export const MAX_FIND_HITS = 2000;

/** Offsets of every non-overlapping occurrence of `query` in `text`, oldest first. */
export function matchOffsets(text: string, query: string, caseSensitive: boolean, cap = MAX_FIND_HITS): number[] {
  if (query === "") return [];
  const haystack = caseSensitive ? text : text.toLowerCase();
  const needle = caseSensitive ? query : query.toLowerCase();
  const out: number[] = [];
  let from = 0;
  while (out.length < cap) {
    const at = haystack.indexOf(needle, from);
    if (at < 0) break;
    out.push(at);
    from = at + Math.max(needle.length, 1);
  }
  return out;
}

export function textMatches(text: string, query: string, caseSensitive: boolean): boolean {
  if (query === "") return false;
  return caseSensitive ? text.includes(query) : text.toLowerCase().includes(query.toLowerCase());
}

/** The index a step lands on: wraps at both ends, and stays put when there is nothing to step. */
export function steppedIndex(current: number, count: number, direction: 1 | -1): number {
  if (count <= 0) return -1;
  if (current < 0) return direction === 1 ? 0 : count - 1;
  return (current + direction + count) % count;
}
