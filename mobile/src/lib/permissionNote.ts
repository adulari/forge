import type { RemoteInput } from "./ws";

export type AllowInput = Extract<RemoteInput, { kind: "allow" }>;

/** The allow/deny reply; `note` is sent only when the field is open and has text, so a bare tap stays `{kind,yes,seq}`. */
export function allowInput(yes: boolean, seq: number, note: string, noteOpen: boolean): AllowInput {
  const trimmed = noteOpen ? note.trim() : "";
  return { kind: "allow", yes, seq, ...(trimmed ? { note: trimmed } : {}) };
}
