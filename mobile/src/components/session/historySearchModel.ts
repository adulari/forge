// Presentation rules for the history-search sheet (GET /api/sessions/{id}/history/search).
import type { HistorySearchHit } from "../../lib/api";

export interface SnippetPart {
  text: string;
  match: boolean;
}

/** Splits a host snippet at its `[` `]` match markers so the matched words can be emphasised. */
export function splitSnippet(snippet: string): SnippetPart[] {
  const parts: SnippetPart[] = [];
  let rest = snippet;
  while (rest.length > 0) {
    const open = rest.indexOf("[");
    const close = open === -1 ? -1 : rest.indexOf("]", open + 1);
    if (open === -1 || close === -1) {
      parts.push({ text: rest, match: false });
      break;
    }
    if (open > 0) parts.push({ text: rest.slice(0, open), match: false });
    parts.push({ text: rest.slice(open + 1, close), match: true });
    rest = rest.slice(close + 1);
  }
  return parts.filter((part) => part.text.length > 0);
}

const ROLE_LABELS: Record<string, string> = {
  user: "You",
  assistant: "Forge",
  tool: "Tool result",
  system: "System",
};

export function roleLabel(role: string): string {
  return ROLE_LABELS[role] ?? role;
}

/** Hits to show: the host's order (best match first), at most `limit`. */
export function visibleHits(hits: readonly HistorySearchHit[], limit = 30): HistorySearchHit[] {
  return hits.slice(0, limit);
}

/** What the sheet says when nothing matched. */
export function emptyMessage(query: string): string {
  return query.trim().length < 2
    ? "Type at least two letters to search this session's whole history, including messages folded out of the model's context."
    : `Nothing in this session matches "${query.trim()}".`;
}
