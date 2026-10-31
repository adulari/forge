import { useSyncExternalStore } from "react";

import { type PreviewConsoleLevel, type PreviewConsoleLine } from "./browserPreview";

export interface PreviewConsoleEntry extends PreviewConsoleLine {
  id: number;
}

export const CONSOLE_LIMIT = 300;
const EMPTY: readonly PreviewConsoleEntry[] = [];
const byLabel = new Map<string, readonly PreviewConsoleEntry[]>();
const listeners = new Set<() => void>();
let sequence = 1;

function emit(): void {
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getConsoleEntries(label: string): readonly PreviewConsoleEntry[] {
  return byLabel.get(label) ?? EMPTY;
}

export function useConsoleEntries(label: string): readonly PreviewConsoleEntry[] {
  return useSyncExternalStore(subscribe, () => getConsoleEntries(label), () => getConsoleEntries(label));
}

/** Keeps the newest `CONSOLE_LIMIT` lines per preview, so a chatty page cannot grow memory. */
export function appendConsoleEntries(label: string, lines: readonly PreviewConsoleLine[]): void {
  if (lines.length === 0) return;
  const added = lines.map((line) => ({ ...line, id: sequence++ }));
  const next = [...getConsoleEntries(label), ...added];
  byLabel.set(label, next.length > CONSOLE_LIMIT ? next.slice(next.length - CONSOLE_LIMIT) : next);
  emit();
}

export function clearConsole(label: string): void {
  if (!byLabel.delete(label)) return;
  emit();
}

export function consoleCounts(entries: readonly PreviewConsoleEntry[]): Record<"error" | "warn", number> {
  let error = 0;
  let warn = 0;
  for (const entry of entries) {
    if (entry.level === "error") error += 1;
    else if (entry.level === "warn") warn += 1;
  }
  return { error, warn };
}

export function isConsoleProblem(level: PreviewConsoleLevel): boolean {
  return level === "error" || level === "warn";
}

export const __consoleTesting = {
  reset(): void {
    byLabel.clear();
  },
};
