// Pure helpers for the duel surfaces: the `/duel` command line a launcher builds, and the
// judge fields the daemon appends to the winner picker (see crates/forge-cli/.../duel_flow.rs).
import type { Overlay } from "../../lib/ws";
import type { DuelPane } from "./DuelView";


/** A remote client puts this in front of a branch id to open a PR from it instead of merging. */
export const OPEN_PR_PREFIX = "pr:";

export const MIN_CANDIDATES = 2;
export const MAX_CANDIDATES = 5;
export const DEFAULT_CANDIDATES = 3;

export interface DuelLaunchOptions {
  judge: boolean;
  /** Number of models to race; omitted from the command when it is the default. */
  candidates: number;
}

export function duelCommand(task: string, { judge, candidates }: DuelLaunchOptions): string {
  const flags: string[] = [];
  if (judge) flags.push("--judge");
  const n = Math.max(MIN_CANDIDATES, Math.min(MAX_CANDIDATES, Math.round(candidates)));
  if (n !== DEFAULT_CANDIDATES) flags.push(`--n ${n}`);
  return ["/duel", ...flags, task.trim()].join(" ");
}

export interface JudgeNote {
  rank: number;
  score: number;
  critique: string;
}

const JUDGE_FIELD = /^judge #(\d+) (\d+(?:\.\d+)?)\/10: ?([\s\S]*)$/;

/** Peels the `judge #N S/10: critique` field off a row's ` · `-separated detail, wherever it sits. */
export function splitJudgeField(parts: string[]): { parts: string[]; judge?: JudgeNote } {
  const at = parts.findIndex((part) => JUDGE_FIELD.test(part.trim()));
  if (at < 0) return { parts };
  const match = JUDGE_FIELD.exec(parts[at].trim()) as RegExpExecArray;
  return {
    parts: parts.filter((_, i) => i !== at),
    judge: { rank: Number(match[1]), score: Number(match[2]), critique: match[3].trim() },
  };
}

export interface DuelHeading {
  judgeModel?: string;
  rationale?: string;
  judgeError?: string;
}

/** Reads the verdict the picker heading carries after ` · judge `. */
export function parseDuelHeading(title: string): DuelHeading {
  const at = title.indexOf(" · judge ");
  if (at < 0) return {};
  const rest = title.slice(at + " · judge ".length);
  if (rest.startsWith("failed: ")) return { judgeError: rest.slice("failed: ".length).trim() };
  const colon = rest.indexOf(": ");
  if (colon < 0) return { judgeModel: rest.trim() };
  return { judgeModel: rest.slice(0, colon).trim(), rationale: rest.slice(colon + 2).trim() };
}

/** Row labels are `[★ ]✓|✗ model`; the crown marks the judge's pick. */
export function parseRowLabel(label: string): { model: string; crowned: boolean } {
  const crowned = label.trimStart().startsWith("★");
  const model = label.replace(/^[★✓✗×\s]+/u, "").trim() || label;
  return { model, crowned };
}

const DETAIL_DIFF_RE = /(\d+)\s+files?\s+\+(\d+)\s+-(\d+)/;

export function panesFromOverlay(overlay: Overlay): DuelPane[] {
  return overlay.rows.map((row, index) => {
    const { model, crowned } = parseRowLabel(row.label);
    const { parts, judge } = splitJudgeField(row.detail.split(" · "));
    const diffMatch = DETAIL_DIFF_RE.exec(parts[0] ?? "");
    const diffstat = diffMatch
      ? { files: Number(diffMatch[1]), added: Number(diffMatch[2]), removed: Number(diffMatch[3]) }
      : undefined;
    const tests = parts.some((p) => /tests\s*✓/.test(p))
      ? true
      : parts.some((p) => /tests\s*✗/.test(p))
        ? false
        : null;
    const latency = Number((parts.find((p) => /^\s*[\d.]+s$/.test(p)) ?? "0s").replace(/s\s*$/, "")) || 0;
    const cost = Number((parts.find((p) => /^\s*\$/.test(p)) ?? "$0").replace(/[^0-9.]/g, "")) || 0;
    const summary = parts.slice(4).join(" · ").trim();
    return {
      id: row.id,
      model,
      costUsd: cost,
      latencySec: latency,
      body: summary || "_(no summary provided)_",
      tone: index === 0 ? "accent" : "info",
      diffstat,
      tests,
      judge,
      crowned: crowned || judge?.rank === 1,
    } satisfies DuelPane;
  });
}

