import type { TurnEndCount } from "./api";
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

/** Ends where sending `continue` picks the work back up (the host's `is_resumable`). */
const RESUMABLE = new Set([
  "stalled",
  "tasks_unfinished",
  "provider_down",
  "provider_error",
  "interrupted",
  "daemon_restart_or_crash",
  "final_answer_mid_intent",
]);

/** Whether the stop is one `continue` fixes: the host's word when it sends one, else the list. */
export function isResumable(end: Pick<TurnEnd, "reason"> & { resumable?: boolean }): boolean {
  return end.resumable ?? RESUMABLE.has(end.reason);
}

/** The banner to show: the last stop, gone as soon as a new turn is running. */
export function visibleTurnEnd(end: TurnEnd | null | undefined, busy: boolean | undefined): TurnEnd | null {
  return end && !busy ? end : null;
}

/** The prompt the banner's action sends, or null when the stop needs something else first. */
export function continuePrompt(end: TurnEnd): string | null {
  return isResumable(end) ? "continue" : null;
}

/** What the person does about it: the host's advice, else a generic line for the reason. */
export function turnEndAdvice(end: TurnEnd): string {
  const advice = end.advice?.trim();
  if (advice) return advice.charAt(0).toUpperCase() + advice.slice(1) + (/[.!?]$/.test(advice) ? "" : ".");
  return isResumable(end) ? "Send continue to pick it up." : "Check the settings, then send your next message.";
}

/** "3 min ago" style age for a unix-seconds timestamp. */
export function endedAgo(at: number, nowSecs: number): string {
  const secs = Math.max(0, nowSecs - at);
  if (secs < 60) return "just now";
  if (secs < 3600) return `${Math.floor(secs / 60)} min ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)} h ago`;
  return `${Math.floor(secs / 86400)} d ago`;
}

/** "12 turns: 9 finished, 2 no progress" from the host's labelled counts. */
export function countsLine(counts: readonly TurnEndCount[]): string {
  const total = counts.reduce((sum, c) => sum + c.count, 0);
  if (total === 0) return "No turns recorded.";
  const parts = counts.slice(0, 4).map((c) => `${c.count} ${c.label}`);
  const more = counts.length - parts.length;
  return `${total} turns: ${parts.join(", ")}${more > 0 ? `, ${more} more kinds` : ""}`;
}
