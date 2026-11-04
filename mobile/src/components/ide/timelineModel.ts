// The agent panel's compact timeline as plain data: the last reads, edits and commands, newest
// first, with the file an edit touched resolved against the project's changed files.
import type { HistoryRow } from "../../lib/api";
import { buildTranscript, parseArgs, resultFailed, summarizeToolArgs, type TranscriptEntry } from "../../lib/toolRows";

export type TimelineKind = "prompt" | "read" | "edit" | "command" | "tool" | "reply";

export interface TimelineItem {
  key: string;
  kind: TimelineKind;
  /** One short line: the target, the command, or the first line of the message. */
  label: string;
  /** What the call was (write_file, shell, ...): shown muted beside the label. */
  tool: string | null;
  failed: boolean;
  adds: number;
  dels: number;
  /** The path an edit touched, as the model wrote it (often absolute). */
  path: string | null;
}

const EDIT = /(^|_)(write|edit|patch|apply|create|replace|multiedit)/i;
const READ = /(^|_)(read|view|cat|ls|list|glob|grep|search|find|open)/i;
const COMMAND = /(^|_)(shell|bash|exec|run|terminal|command|test)/i;

export function toolKind(tool: string | null): TimelineKind {
  if (!tool) return "tool";
  if (EDIT.test(tool)) return "edit";
  if (COMMAND.test(tool)) return "command";
  if (READ.test(tool)) return "read";
  return "tool";
}

function firstLine(text: string, max = 90): string {
  const line = text.split("\n").find((candidate) => candidate.trim().length > 0)?.trim() ?? "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

function editPath(args: string): string | null {
  const parsed = parseArgs(args);
  if (!parsed) return null;
  for (const key of ["path", "file_path", "file", "filepath"]) {
    const value = parsed[key];
    if (typeof value === "string" && value) return value;
  }
  return null;
}

/** Input and output are newest first, the order the chat's inverted list uses and the panel reads. */
export function buildTimeline(rowsNewestFirst: readonly HistoryRow[], limit = 60): TimelineItem[] {
  const entries: TranscriptEntry[] = buildTranscript([...rowsNewestFirst]);
  const out: TimelineItem[] = [];
  for (const entry of entries) {
    if (out.length >= limit) break;
    if (entry.kind === "tool") {
      const { invocation } = entry;
      const kind = toolKind(invocation.tool);
      out.push({
        key: entry.key,
        kind,
        label: summarizeToolArgs(invocation.args, 52) || invocation.tool || "tool",
        tool: invocation.tool,
        failed: resultFailed(invocation.result),
        adds: invocation.adds ?? 0,
        dels: invocation.dels ?? 0,
        path: kind === "edit" ? editPath(invocation.args) : null,
      });
      continue;
    }
    const { row } = entry;
    const text = firstLine(row.content);
    if (!text) continue;
    if (row.role === "user" && (row.kind == null || row.kind === "user")) {
      out.push({ key: entry.key, kind: "prompt", label: text, tool: null, failed: false, adds: 0, dels: 0, path: null });
    } else if (row.role === "assistant" && row.visibility !== "ui") {
      out.push({ key: entry.key, kind: "reply", label: text, tool: null, failed: false, adds: 0, dels: 0, path: null });
    }
  }
  return out;
}

/** A model-written path (often absolute) → the changed file it means, by whole path segments. */
export function resolveChangedPath(written: string, changed: readonly string[]): string | null {
  const normalized = written.replace(/^\.\//, "");
  if (changed.includes(normalized)) return normalized;
  let best: string | null = null;
  for (const candidate of changed) {
    if (normalized.endsWith(`/${candidate}`) && (best == null || candidate.length > best.length)) best = candidate;
  }
  return best;
}
