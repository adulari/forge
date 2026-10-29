export type FocusTag = "composerFocus" | "terminalFocus" | "sidebarFocus" | "editorFocus" | "hasSession";

export const EXCLUSIVE_FOCUS_TAGS: readonly FocusTag[] = [
  "composerFocus",
  "terminalFocus",
  "sidebarFocus",
  "editorFocus",
];

export interface ParsedWhen {
  pos: Set<string>;
  neg: Set<string>;
}

export function parseWhen(when: string | undefined): ParsedWhen {
  const parsed: ParsedWhen = { pos: new Set(), neg: new Set() };
  for (const raw of (when ?? "").split("&&")) {
    const term = raw.trim();
    if (!term) continue;
    if (term.startsWith("!")) parsed.neg.add(term.slice(1).trim());
    else parsed.pos.add(term);
  }
  return parsed;
}

export function evaluateWhen(when: string | undefined, tags: ReadonlySet<string>): boolean {
  const { pos, neg } = parseWhen(when);
  for (const tag of pos) if (!tags.has(tag)) return false;
  for (const tag of neg) if (tags.has(tag)) return false;
  return true;
}

/** True when no focus state can satisfy both expressions, so a chord may be reused. */
export function whenDisjoint(left: string | undefined, right: string | undefined): boolean {
  const a = parseWhen(left);
  const b = parseWhen(right);
  for (const tag of a.pos) if (b.neg.has(tag)) return true;
  for (const tag of b.pos) if (a.neg.has(tag)) return true;
  const exclusive = (side: ParsedWhen) => [...side.pos].filter((tag) => EXCLUSIVE_FOCUS_TAGS.includes(tag as FocusTag));
  const ax = exclusive(a);
  const bx = exclusive(b);
  return ax.length > 0 && bx.length > 0 && !ax.some((tag) => bx.includes(tag));
}

const SCOPE_NOTES: Record<string, string> = {
  composerFocus: "the composer is focused",
  sidebarFocus: "a sidebar row is focused",
  terminalFocus: "the terminal is focused",
  editorFocus: "the editor is focused",
  hasSession: "a session is open",
};

export function describeWhen(when: string | undefined): string | null {
  if (!when) return null;
  const { pos, neg } = parseWhen(when);
  const parts = [
    ...[...pos].map((tag) => SCOPE_NOTES[tag] ?? tag),
    ...[...neg].map((tag) => `${SCOPE_NOTES[tag] ?? tag}`.replace(/ is /, " is not ")),
  ];
  return parts.length ? `Only when ${parts.join(" and ")}` : null;
}
