// Tool activity in the transcript: one-line summarization (shared by the chat's tool card and
// the legacy `system`-row body) plus the call→result pairing an `include_tools` history page
// needs before it can render.
//
// The daemon serves a tool CALL and its RESULT as two separate rows (`tool_phase`), which reads
// as duplicated noise in a chat timeline — one invocation should be one row. `buildTranscript`
// collapses them back into a single entry per invocation.
import type { HistoryRow, TranscriptKind } from "./api";

// Preference order for the "primary target" pulled out of a tool call's JSON args — the first
// of these present as a non-empty string wins. Covers the common shapes across Forge's tool
// surface (file ops, shell, search) without needing per-tool-name special cases.
const TARGET_KEYS = [
  "path",
  "file",
  "filepath",
  "file_path",
  "command",
  "cmd",
  "script",
  "query",
  "pattern",
  "url",
  "prompt",
  "action",
];

export function truncateMiddle(value: string, max = 44): string {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${trimmed.slice(0, head)}…${trimmed.slice(trimmed.length - tail)}`;
}

/** Parses a tool-call argument blob. The store caps long values with a trailing `…`, which can
 * leave the JSON unparsable — that is expected, not an error, and yields `null`. */
export function parseArgs(argsJson: string): Record<string, unknown> | null {
  const text = argsJson.trim();
  if (!text.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(text) as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

/**
 * The human-readable target of a call, from its JSON args — never the raw blob. Returns "" when
 * nothing recognizable is present, so the caller renders the bare tool name instead of leaking
 * arguments into the summary line.
 */
export function summarizeToolArgs(argsJson: string, max = 44): string {
  const parsed = parseArgs(argsJson);
  if (!parsed) return "";
  for (const key of TARGET_KEYS) {
    const value = parsed[key];
    if (typeof value === "string" && value.trim().length > 0) return truncateMiddle(value, max);
  }
  return "";
}

/** Pretty-prints a call's arguments for the expanded card. Falls back to the raw text when the
 * blob doesn't parse (a store-capped value), which is still more useful than nothing. */
export function formatArgs(argsJson: string): string {
  const parsed = parseArgs(argsJson);
  if (!parsed) return argsJson.trim();
  return JSON.stringify(parsed, null, 2);
}

/**
 * Turns a transcript/tool-call line (e.g. `↳ write_file {"content":"…","cwd":"…"}`) into a
 * compact one-line summary. A line with no `{…}` blob is already clean and passes through
 * unchanged; a JSON blob is parsed for a recognizable target field instead of ever being
 * rendered verbatim — falls back to the bare tool name (never the raw args) if nothing
 * recognizable is found or the blob doesn't parse.
 */
export function summarizeToolLine(rawLine: string): string {
  const line = rawLine.replace(/^↳\s*/, "").trim();
  const braceIdx = line.indexOf("{");
  if (braceIdx === -1) return line;
  const name = line.slice(0, braceIdx).trim().replace(/[:(]+$/, "") || "tool";
  let argsText = line.slice(braceIdx);
  if (argsText.endsWith(")")) argsText = argsText.slice(0, -1);
  const target = summarizeToolArgs(argsText);
  return target ? `${name} ${target}` : name;
}

/** A tool result whose text reads as a failure. Heuristic — a PERSISTED result row carries no
 * outcome flag (remote.rs `tool_phase` doc), so text is the only signal available. */
export function resultFailed(result: string | null): boolean {
  if (!result) return false;
  const head = result.slice(0, 400);
  return /\b(failed|error)\b/i.test(head) && !/\bpassed\b/i.test(head);
}

export interface ToolInvocation {
  seq: number;
  tool: string | null;
  /** The JSON argument blob the model sent. "" when only a result row survives. */
  args: string;
  /** What came back. `null` while the call is still in flight or its result was never stored. */
  result: string | null;
}

export type TranscriptEntry =
  | { kind: "message"; key: string; row: HistoryRow }
  | { kind: "tool"; key: string; invocation: ToolInvocation };

/** A row's provenance, falling back to `role` on a pre-v9 daemon that sends no `kind`. */
export function rowKind(row: HistoryRow): TranscriptKind {
  if (row.kind) return row.kind;
  if (row.role === "user") return "user";
  if (row.role === "assistant") return row.visibility === "ui" ? "system" : "assistant";
  if (row.role === "tool") return "tool";
  return "system";
}

/**
 * Collapses an `include_tools` history page into render entries, pairing each tool call with the
 * result that answered it so one invocation is one row.
 *
 * Input and output are both NEWEST-FIRST (what the inverted chat list consumes). Pairing runs
 * chronologically because that is the only order in which "the next result for this tool answers
 * this call" holds. A result with no preceding call (its carrier is no longer recoverable, so the
 * store could not name the tool) still renders — as an argument-less entry, never dropped.
 */
export function buildTranscript(rowsNewestFirst: HistoryRow[]): TranscriptEntry[] {
  const out: TranscriptEntry[] = [];
  // Index into `out` of the newest unanswered call per tool name.
  const awaiting = new Map<string, number>();
  let n = 0;
  for (let i = rowsNewestFirst.length - 1; i >= 0; i--) {
    const row = rowsNewestFirst[i];
    if (rowKind(row) !== "tool") {
      out.push({ kind: "message", key: `h${row.seq}-${n++}`, row });
      continue;
    }
    const name = row.tool ?? "";
    // A tool row with no phase (pre-v10 daemon) is a result — that is the only tool row those
    // daemons ever served.
    if (row.tool_phase === "call") {
      out.push({
        kind: "tool",
        key: `t${row.seq}-${n++}`,
        invocation: { seq: row.seq, tool: row.tool ?? null, args: row.content, result: null },
      });
      awaiting.set(name, out.length - 1);
      continue;
    }
    const at = awaiting.get(name);
    const pending = at === undefined ? undefined : out[at];
    if (pending && pending.kind === "tool" && pending.invocation.result === null) {
      pending.invocation.result = row.content;
      awaiting.delete(name);
      continue;
    }
    out.push({
      kind: "tool",
      key: `t${row.seq}-${n++}`,
      invocation: { seq: row.seq, tool: row.tool ?? null, args: "", result: row.content },
    });
  }
  return out.reverse();
}
