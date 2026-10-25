// The session header's title. A session the daemon never titled used to show its raw id
// ("session db89c45e"), which says nothing about what it is for; the first thing the user asked
// it is a better name until a real title exists.
import type { HistoryRow } from "./api";

const MAX_TITLE = 72;
const UPLOAD_MENTION = /@\S*\.forge\/uploads\/\S+\s*/g;

function shorten(text: string): string {
  const line = text.replace(UPLOAD_MENTION, "").replace(/\s+/g, " ").trim();
  return line.length > MAX_TITLE ? `${line.slice(0, MAX_TITLE - 1).trimEnd()}…` : line;
}

/** `rowsNewestFirst` is the loaded history; its oldest user turn names the session. */
export function sessionDisplayTitle(title: string | null | undefined, rowsNewestFirst: readonly HistoryRow[], sessionId: string): string {
  if (title?.trim()) return title.trim();
  for (let i = rowsNewestFirst.length - 1; i >= 0; i--) {
    const row = rowsNewestFirst[i];
    if (row.role !== "user" || (row.kind != null && row.kind !== "user")) continue;
    const text = shorten(row.content);
    if (text) return text;
  }
  return `session ${sessionId.slice(0, 8)}`;
}
