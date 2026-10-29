import { splitPathRefs, type PathRef } from "../../lib/pathRefs";

export type TerminalLinkTarget =
  | { kind: "url"; url: string }
  | { kind: "path"; ref: PathRef };

export type TerminalLinkPart =
  | { type: "text"; text: string }
  | { type: "link"; text: string; target: TerminalLinkTarget };

const URL_PATTERN = /https?:\/\/[^\s<>"'`\x00-\x1f]+/gi;
const CLOSERS: Record<string, string> = { ")": "(", "]": "[", "}": "{" };

function count(text: string, char: string): number {
  return text.split(char).length - 1;
}

/** Sentence punctuation after a URL is not part of it; a closing bracket is only when it was opened. */
function trimUrl(raw: string): string {
  let url = raw;
  while (url.length > 0) {
    const last = url[url.length - 1];
    const opener = CLOSERS[last];
    if (/[.,;:!?'">]/.test(last) || (opener && count(url, last) > count(url, opener))) {
      url = url.slice(0, -1);
      continue;
    }
    break;
  }
  return url;
}

/**
 * A terminal path is only worth a link when it points somewhere: it names a line (`src/a.ts:5`)
 * or sits in a folder. A bare `Cargo.toml` in `ls` output is a listing, not a reference.
 */
function worthLinking(ref: PathRef): boolean {
  return ref.line != null || ref.path.includes("/");
}

function pathParts(text: string): TerminalLinkPart[] {
  const parts: TerminalLinkPart[] = [];
  for (const part of splitPathRefs(text)) {
    if (part.type === "path" && worthLinking(part.ref)) {
      parts.push({ type: "link", text: part.text, target: { kind: "path", ref: part.ref } });
    } else {
      parts.push({ type: "text", text: part.text });
    }
  }
  return parts;
}

/** `text` cut into plain runs, http(s) URLs and `path:line` references, in order. */
export function detectTerminalLinks(text: string): TerminalLinkPart[] {
  if (!text.includes("/") && !text.includes(".")) return [{ type: "text", text }];
  const parts: TerminalLinkPart[] = [];
  let last = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    const url = trimUrl(match[0]);
    if (url.length <= "https://".length) continue;
    if (match.index > last) parts.push(...pathParts(text.slice(last, match.index)));
    parts.push({ type: "link", text: url, target: { kind: "url", url } });
    last = match.index + url.length;
  }
  if (last < text.length) parts.push(...pathParts(text.slice(last)));
  return parts.length > 0 ? parts : [{ type: "text", text }];
}
