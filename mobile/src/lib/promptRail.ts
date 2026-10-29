// The prompt rail: one tick per user prompt down the right gutter of a long transcript, placed by
// where the prompt sits in the session, so it works as an outline even for prompts that were never
// paged in.
import type { HistoryRow } from "./api";

export interface RailPrompt {
  seq: number;
  text: string;
  /** When it was sent, epoch seconds. */
  at: number;
}

export interface RailTick extends RailPrompt {
  /** 0 at the oldest prompt, 1 at the newest. */
  top: number;
}

/** A real user message, not a harness nudge or a note. */
export function isRailPrompt(row: HistoryRow): boolean {
  return row.role === "user" && row.kind !== "nudge" && row.visibility === "llm";
}

/** Prompts from the host's outline and the loaded rows, de-duplicated by seq, oldest first. */
export function mergePrompts(
  remote: readonly HistoryRow[],
  loaded: readonly HistoryRow[],
  textOf: (row: HistoryRow) => string,
): RailPrompt[] {
  const bySeq = new Map<number, RailPrompt>();
  for (const row of [...remote, ...loaded]) {
    if (isRailPrompt(row)) bySeq.set(row.seq, { seq: row.seq, text: textOf(row), at: row.created_at });
  }
  return [...bySeq.values()].sort((a, b) => a.seq - b.seq);
}

export function railTicks(prompts: readonly RailPrompt[]): RailTick[] {
  if (prompts.length === 0) return [];
  const first = prompts[0].seq;
  const span = prompts[prompts.length - 1].seq - first;
  return prompts.map((prompt) => ({
    ...prompt,
    top: span > 0 ? (prompt.seq - first) / span : 0.5,
  }));
}

/** The rail needs room, a precise pointer to hover with, and something to navigate. */
export function railVisible(input: { promptCount: number; expanded: boolean; finePointer: boolean }): boolean {
  return input.expanded && input.finePointer && input.promptCount >= 2;
}

/** At most `slots` ticks: prompts that would land within a slot of each other collapse to the
 * newest of them, so a session with hundreds of prompts reads as an outline, not a solid bar. */
export function bucketTicks(ticks: readonly RailTick[], slots = 72): RailTick[] {
  if (ticks.length <= slots) return [...ticks];
  const bySlot = new Map<number, RailTick>();
  for (const tick of ticks) bySlot.set(Math.min(slots - 1, Math.floor(tick.top * slots)), tick);
  return [...bySlot.values()];
}
