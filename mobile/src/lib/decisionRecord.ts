// An answered `ask_user` kept as a decision record in the transcript: what was asked, what was
// chosen. Both halves are already persisted — the call's arguments (the questions and their
// options) and its result (the answers, rendered by the daemon's `render_answers`: a bare summary
// for one question, `N. header: summary` lines for a form) — so the record survives a reload with
// no new storage.
import { parseArgs, resultFailed, type ToolInvocation } from "./toolRows";

export interface DecisionItem {
  header: string;
  question: string;
  /** What was picked, as the daemon summarised it: labels, free text, or "". */
  chosen: string;
  /** The picked option's description, only when exactly one option was picked. */
  description: string;
  note: string;
  answered: boolean;
}

const NOTE_SEPARATOR = " — note: ";
const NO_ANSWER = /^\(no answer/i;

export function isDecision(invocation: ToolInvocation): boolean {
  if (invocation.tool == null || invocation.tool.replace(/^mcp__forge__/, "") !== "ask_user") return false;
  return invocation.result !== null && invocation.result.trim() !== "" && !resultFailed(invocation.result);
}

interface AskedQuestion {
  header: string;
  question: string;
  options: { label: string; description: string }[];
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function askedQuestions(args: string): AskedQuestion[] {
  const parsed = parseArgs(args);
  if (!parsed) {
    const grab = (key: string) => {
      const m = new RegExp(`"${key}"\\s*:\\s*("(?:[^"\\\\]|\\\\.)*")`).exec(args);
      try {
        return m ? text(JSON.parse(m[1])) : "";
      } catch {
        return "";
      }
    };
    return [{ header: grab("header"), question: grab("question") || grab("text"), options: [] }];
  }
  const list = Array.isArray(parsed.questions) ? (parsed.questions as unknown[]) : [parsed];
  return list.map((entry) => {
    const q = (entry && typeof entry === "object" ? entry : {}) as Record<string, unknown>;
    const options = Array.isArray(q.options) ? (q.options as unknown[]) : [];
    return {
      header: text(q.header),
      question: text(q.question) || text(q.text),
      options: options
        .map((o) => (typeof o === "string" ? { label: o.trim(), description: "" } : { label: text((o as { label?: unknown })?.label), description: text((o as { description?: unknown })?.description) }))
        .filter((o) => o.label),
    };
  });
}

function splitNote(summary: string): { chosen: string; note: string } {
  const at = summary.indexOf(NOTE_SEPARATOR);
  return at < 0 ? { chosen: summary.trim(), note: "" } : { chosen: summary.slice(0, at).trim(), note: summary.slice(at + NOTE_SEPARATOR.length).trim() };
}

function descriptionOf(asked: AskedQuestion, chosen: string): string {
  const picked = asked.options.filter((o) => chosen.split(", ").includes(o.label));
  return picked.length === 1 ? picked[0].description : "";
}

export function parseDecision(invocation: ToolInvocation): DecisionItem[] {
  const result = (invocation.result ?? "").trim();
  const asked = askedQuestions(invocation.args);
  const empty = NO_ANSWER.test(result);
  const numbered = result.split("\n").map((line) => /^(\d+)\.\s+(.*?):\s(.*)$/.exec(line));
  const isForm = asked.length > 1 && numbered.every((m) => m !== null);
  return asked.map((q, index) => {
    const raw = empty ? "" : isForm ? (numbered[index]?.[3] ?? "") : asked.length === 1 ? result : "";
    const { chosen, note } = splitNote(raw);
    const answered = chosen !== "" && !NO_ANSWER.test(chosen);
    return {
      header: q.header,
      question: q.question,
      chosen: answered ? chosen : "",
      description: answered ? descriptionOf(q, chosen) : "",
      note,
      answered,
    };
  });
}
