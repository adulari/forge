import { highlightTokens, type HighlightToken } from "../../lib/highlightTokens";

/** Past this size the tokenizer is skipped: a minified bundle would block the UI thread. */
export const HIGHLIGHT_LIMIT_CHARS = 300_000;

/** Highlight `code`, then cut the token stream at every newline so each line renders on its own. */
export function tokensByLine(code: string, language: string): HighlightToken[][] {
  const tokens: HighlightToken[] =
    code.length > HIGHLIGHT_LIMIT_CHARS ? [{ kind: "plain", text: code }] : highlightTokens(code, language);
  const lines: HighlightToken[][] = [[]];
  for (const token of tokens) {
    const parts = token.text.split("\n");
    parts.forEach((part, index) => {
      if (index > 0) lines.push([]);
      if (part) lines[lines.length - 1]?.push({ kind: token.kind, text: part });
    });
  }
  return lines;
}

/** Width in characters of the gutter for `count` lines. */
export function gutterWidth(count: number): number {
  return Math.max(2, String(count).length);
}
