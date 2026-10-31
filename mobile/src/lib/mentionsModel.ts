// @-mentions that pull other context into a prompt: `@session:<name>`, `@terminal`, `@browser`
// and `@Subagents`. The daemon (and the TUI) expand them into bounded digests when the prompt is
// sent; this file is the composer's half: which options the `@` menu offers, which tokens a draft
// already holds (rendered as chips) and how a chip is taken back out. The grammar mirrors
// `crates/forge-cli/src/mentions.rs`.

export type MentionKind = "session" | "terminal" | "browser" | "subagents";

export interface MentionOption {
  /** The text after `@`, as the composer's file menu inserts it (`terminal`, `session:auth fix`). */
  path: string;
  kind: MentionKind;
  label: string;
  detail: string;
}

export interface MentionSession {
  id: string;
  title: string | null;
}

export interface DraftMention {
  kind: MentionKind;
  /** The token as typed, `@` included. */
  token: string;
  label: string;
}

const SPECIALS: readonly MentionOption[] = [
  { path: "terminal", kind: "terminal", label: "@terminal", detail: "last lines of this session's terminal" },
  { path: "browser", kind: "browser", label: "@browser", detail: "latest browser screenshot" },
  { path: "Subagents", kind: "subagents", label: "@Subagents", detail: "this session's background subagents" },
];

const MAX_SESSION_OPTIONS = 6;

export function isMentionOption(value: unknown): value is MentionOption {
  return typeof value === "object" && value !== null && "kind" in value && "label" in value && !("line" in value);
}

/** What the `@` menu offers for the text typed after `@`. Empty when nothing applies, so a file
 *  query like `src/ma` stays a plain file search. */
export function mentionOptions(query: string, sessions: readonly MentionSession[], selfId: string | null): MentionOption[] {
  const q = query.toLowerCase();
  const out: MentionOption[] = [];
  if (!q.includes("/")) {
    for (const special of SPECIALS) {
      if (special.path.toLowerCase().startsWith(q)) out.push(special);
    }
    const sessionPrefix = "session:";
    const wantsSessions = q.startsWith(sessionPrefix) || (q.length > 0 && sessionPrefix.startsWith(q));
    if (wantsSessions) {
      const needle = q.startsWith(sessionPrefix) ? q.slice(sessionPrefix.length) : "";
      let count = 0;
      for (const session of sessions) {
        const title = session.title?.trim();
        if (!title || session.id === selfId) continue;
        if (needle && !title.toLowerCase().includes(needle)) continue;
        out.push({ path: `session:${title}`, kind: "session", label: `@session:${title}`, detail: "recent conversation" });
        if (++count >= MAX_SESSION_OPTIONS) break;
      }
    }
  }
  return out;
}

const TRAILING = /[,.;:!?)]+$/;

/** The mentions a draft holds, once each, in order. */
export function draftMentions(text: string): DraftMention[] {
  const found: DraftMention[] = [];
  const seen = new Set<string>();
  const add = (kind: MentionKind, token: string, label: string) => {
    if (seen.has(token)) return;
    seen.add(token);
    found.push({ kind, token, label });
  };
  const pattern = /(^|\s)@(session:\{[^}]+\}|session:[^\s{}]+|terminal(?::[^\s]+)?|browser|subagents)/gi;
  for (const match of text.matchAll(pattern)) {
    const raw = (match[2] ?? "").replace(TRAILING, "");
    const lower = raw.toLowerCase();
    if (lower.startsWith("session:")) {
      const name = raw.slice("session:".length).replace(/^\{|\}$/g, "");
      if (name) add("session", `@${raw}`, name);
    } else if (lower.startsWith("terminal")) {
      add("terminal", `@${raw}`, "terminal");
    } else if (lower === "browser") {
      add("browser", `@${raw}`, "browser");
    } else if (lower === "subagents") {
      add("subagents", `@${raw}`, "Subagents");
    }
  }
  return found;
}

/** The draft without one mention token (and the space after it). */
export function removeMention(text: string, token: string): string {
  const at = text.indexOf(token);
  if (at < 0) return text;
  const after = text.slice(at + token.length).replace(/^ /, "");
  return `${text.slice(0, at)}${after}`;
}
