// A shell command as a person would title it: the wrapper a model puts around every call
// (`bash -lc '...'`, a leading `cd dir &&`) is noise, and a `sed -n 'a,bp' file` is a view of
// lines a to b. The row shows "View src/a.ts 10-40", not the incantation.
import { friendlyCommand, shortenPath } from "./pathDisplay";

const SHELL_WRAPPER = /^(?:\/(?:usr\/)?bin\/)?(?:bash|sh|zsh)\s+-[a-z]*c\s+(.+)$/s;
const LEADING_CD = /^cd\s+(?:"[^"]*"|'[^']*'|[^\s&;|]+)\s*(?:&&|;)\s*/;
const LEADING_SETUP = /^(?:set\s+-[a-z]+(?:\s+-o\s+\w+)?|export\s+\w+=\S*)\s*(?:&&|;)\s*/;
const SED_VIEW = /^sed\s+-n\s+(['"]?)(\d+)(?:,(\d+))?p\1\s+(?:--\s+)?(\S+)$/;

function unquote(text: string): string {
  const t = text.trim();
  const quote = t[0];
  if ((quote === "'" || quote === '"') && t.length >= 2 && t.endsWith(quote)) {
    const inner = t.slice(1, -1);
    return quote === "'" ? inner.replace(/'\\''/g, "'") : inner.replace(/\\(["\\$`])/g, "$1");
  }
  // A stored command is capped, which can cut the closing quote off: the opening one still goes.
  if (quote === "'" || quote === '"') return t.slice(1);
  return t;
}

function strip(command: string): string {
  let text = command.trim();
  for (let guard = 0; guard < 6; guard++) {
    const wrapped = SHELL_WRAPPER.exec(text);
    if (wrapped) {
      text = unquote(wrapped[1]);
      continue;
    }
    const next = text.replace(LEADING_CD, "").replace(LEADING_SETUP, "").replace(/^exec\s+/, "");
    if (next === text) break;
    text = next.trim();
  }
  return text;
}

export function commandSummary(command: string): string {
  const text = strip(command);
  const view = SED_VIEW.exec(text);
  if (view) {
    const file = unquote(view[4]);
    const from = view[2];
    const to = view[3];
    const range = to && to !== from ? `${from}-${to}` : from;
    return `View ${shortenPath(file)} ${range}`;
  }
  return friendlyCommand(text || command);
}
