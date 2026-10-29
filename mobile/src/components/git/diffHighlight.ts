// Syntax colour for diff lines. `lib/highlightTokens.ts` tokenises one string at a time, so a diff
// is highlighted per line: a block comment that opens on an earlier line is not recognised on the
// next one. That is the honest cost of a line-oriented view; it never misattributes a change,
// because add/delete tinting and the intraline "changed" spans are computed separately and only
// merged here, at the span level.
import { highlightTokens, type TokenKind } from "../../lib/highlightTokens";
import { type DiffTextSegment } from "./diffModel";

const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  rs: "rust",
  ts: "js",
  tsx: "js",
  js: "js",
  jsx: "js",
  mjs: "js",
  cjs: "js",
  py: "python",
  go: "go",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  json: "json",
  jsonc: "json",
};

/** Lines longer than this are shown plain: tokenising a minified bundle line costs more than it
 * helps, and nobody reads it. */
const MAX_HIGHLIGHT_CHARS = 600;

export function languageForPath(path: string): string | null {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  return LANGUAGE_BY_EXTENSION[name.slice(dot + 1).toLowerCase()] ?? null;
}

export interface StyledSpan {
  text: string;
  kind: TokenKind;
  /** Part of the intraline change against the paired line. */
  changed: boolean;
}

/** `text` split at every token and every changed-segment boundary. */
export function styledSpans(
  text: string,
  language: string | null,
  segments?: readonly DiffTextSegment[],
): StyledSpan[] {
  const tokens =
    language && text.length > 0 && text.length <= MAX_HIGHLIGHT_CHARS
      ? highlightTokens(text, language)
      : [{ kind: "plain" as TokenKind, text }];
  const spans = segments?.length ? segments : [{ text, changed: false }];

  const result: StyledSpan[] = [];
  let tokenIndex = 0;
  let tokenOffset = 0;
  let segmentIndex = 0;
  let segmentOffset = 0;
  while (tokenIndex < tokens.length && segmentIndex < spans.length) {
    const token = tokens[tokenIndex];
    const segment = spans[segmentIndex];
    const take = Math.min(token.text.length - tokenOffset, segment.text.length - segmentOffset);
    if (take <= 0) {
      if (tokenOffset >= token.text.length) {
        tokenIndex += 1;
        tokenOffset = 0;
      }
      if (segmentOffset >= segment.text.length) {
        segmentIndex += 1;
        segmentOffset = 0;
      }
      continue;
    }
    const piece = token.text.slice(tokenOffset, tokenOffset + take);
    const previous = result[result.length - 1];
    if (previous && previous.kind === token.kind && previous.changed === segment.changed) {
      previous.text += piece;
    } else {
      result.push({ text: piece, kind: token.kind, changed: segment.changed });
    }
    tokenOffset += take;
    segmentOffset += take;
    if (tokenOffset >= token.text.length) {
      tokenIndex += 1;
      tokenOffset = 0;
    }
    if (segmentOffset >= segment.text.length) {
      segmentIndex += 1;
      segmentOffset = 0;
    }
  }
  return result.length > 0 ? result : [{ text, kind: "plain", changed: false }];
}
