// The faint line closing a finished turn: "Worked for 23s · 4.1k tokens · $0.04". The same three
// facts the terminal prints under a turn (elapsed, tokens, cost), here read from the history the
// daemon already serves: row timestamps for the time, the usage the daemon attaches to each model
// reply for the rest. Pure; the row is components/chat/TurnFooter.
import { rowKind, type ToolInvocation } from "./toolRows";
import type { FoldedEntry } from "./toolGroups";

export interface TurnFooterEntry {
  kind: "turnFooter";
  key: string;
  text: string;
}

export type TimelineEntry = FoldedEntry | TurnFooterEntry;

export function formatWorked(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  if (total < 60) return `${total}s`;
  const minutes = Math.floor(total / 60);
  const rest = total % 60;
  if (minutes < 60) return `${minutes}m ${String(rest).padStart(2, "0")}s`;
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

function formatCost(usd: number): string {
  return usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

export function formatTokens(count: number): string {
  if (count < 1000) return String(Math.round(count));
  if (count < 1_000_000) return `${trim(count / 1000)}k`;
  return `${trim(count / 1_000_000)}M`;
}

function trim(value: number): string {
  const rounded = Math.round(value * 10) / 10;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(1);
}

export interface TurnFacts {
  seconds: number;
  tokens: number | null;
  costUsd: number | null;
}

export function turnFooterText(facts: TurnFacts): string {
  const parts: string[] = [];
  if (facts.seconds >= 1) parts.push(`Worked for ${formatWorked(facts.seconds)}`);
  if (facts.tokens != null && facts.tokens > 0) parts.push(`${formatTokens(facts.tokens)} tokens`);
  if (facts.costUsd != null && facts.costUsd > 0) parts.push(formatCost(facts.costUsd));
  return parts.join(" · ");
}

const TURN_STARTS = new Set(["user", "job_notice", "harness_notice"]);

function invocationsOf(entry: FoldedEntry): ToolInvocation[] {
  if (entry.kind === "tool") return [entry.invocation];
  if (entry.kind === "toolGroup") return entry.calls.map((c) => c.invocation);
  return [];
}

interface Tally {
  start: number;
  end: number;
  input: number;
  output: number;
  cost: number;
  metered: boolean;
  worked: boolean;
}

function startsTurn(entry: Extract<FoldedEntry, { kind: "message" }>): boolean {
  return TURN_STARTS.has(rowKind(entry.row));
}

function newTally(start: number): Tally {
  return { start, end: start, input: 0, output: 0, cost: 0, metered: false, worked: false };
}

function absorb(tally: Tally, entry: FoldedEntry) {
  if (entry.kind === "message") {
    const row = entry.row;
    tally.end = Math.max(tally.end, row.created_at);
    if (rowKind(row) === "assistant") tally.worked = true;
    if (row.input_tokens != null || row.output_tokens != null || row.cost_usd != null) {
      tally.metered = true;
      tally.input += row.input_tokens ?? 0;
      tally.output += row.output_tokens ?? 0;
      tally.cost += row.cost_usd ?? 0;
    }
    return;
  }
  for (const inv of invocationsOf(entry)) {
    tally.worked = true;
    tally.end = Math.max(tally.end, inv.at ?? 0, inv.resultAt ?? 0);
    if (inv.usage) {
      tally.metered = true;
      tally.input += inv.usage.inputTokens;
      tally.output += inv.usage.outputTokens;
      tally.cost += inv.usage.costUsd;
    }
  }
}

/**
 * Closes each finished turn with a footer entry. Input and output are newest-first (what the
 * inverted chat list consumes). A turn is the run of entries from a person's message to the next
 * one; the newest turn is finished only once `busy` is false. A turn whose opening message is not
 * in the loaded page gets no footer: its time and cost would be partial.
 */
export function withTurnFooters(entries: readonly FoldedEntry[], busy: boolean): TimelineEntry[] {
  const out: TimelineEntry[] = [];
  let openSeq: number | null = null;
  let tally = newTally(0);

  const close = (finished: boolean) => {
    if (openSeq === null || !finished || !tally.worked) return;
    const text = turnFooterText({
      seconds: tally.end - tally.start,
      tokens: tally.metered ? tally.input + tally.output : null,
      costUsd: tally.metered ? tally.cost : null,
    });
    if (text) out.push({ kind: "turnFooter", key: `tf${openSeq}`, text });
  };

  for (let i = entries.length - 1; i >= 0; i--) {
    const entry = entries[i];
    if (entry.kind === "message" && startsTurn(entry)) {
      close(true);
      openSeq = entry.row.seq;
      tally = newTally(entry.row.created_at);
    } else if (openSeq !== null) {
      absorb(tally, entry);
    }
    out.push(entry);
  }
  close(!busy);
  return out.reverse();
}
