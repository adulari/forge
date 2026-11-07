import type { Snapshot } from "./ws";

type TurnEnd = NonNullable<Snapshot["last_turn_end"]>;

/** Headline for the dock a turn that stopped short leaves on the session screen. */
export function turnEndTitle(end: TurnEnd): string {
  const label = end.label.trim();
  return `Turn stopped: ${label.length > 0 ? label : end.reason.replace(/_/g, " ")}`;
}

/** Identity of one stop, so dismissing it does not hide a later one. */
export function turnEndKey(end: TurnEnd): string {
  return `${end.reason}|${end.at}`;
}
