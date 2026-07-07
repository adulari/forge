// Reasoning ("thinking") lives INLINE in the same text channel as the answer, delimited by
// `<think>…</think>` tags — verified against the live daemon (protocol v7): reasoning-capable
// models (e.g. nvidia::z-ai/glm-5.2) stream those tags verbatim inside `snapshot.streaming`
// and persist them inside the assistant history row's `content`. The daemon has no separate
// reasoning field/row; its native `reasoning_content` channel is dropped before the snapshot,
// so INLINE markers are the only reasoning that reaches the app. `parseReasoning` splits them
// out so the disclosure gets the thinking text and the main slot gets only the answer.
import { useCallback, useSyncExternalStore } from "react";

export interface ParsedReasoning {
  /** Text inside `<think>…</think>` — may be partial while the block is still streaming. */
  reasoning: string;
  /** Everything OUTSIDE the reasoning block: the actual answer shown in the main slot. */
  answer: string;
  /** True while inside an unclosed `<think>` — reasoning is streaming, the answer hasn't begun. */
  thinking: boolean;
}

const OPEN_RE = /<think(?:ing)?\s*>/i;
const CLOSE_RE = /<\/think(?:ing)?\s*>/i;
// A trailing, not-yet-complete opening tag (`<`, `<t`, … `<thinking`) with no closing `>` yet —
// held back so a half-arrived tag never flashes as answer text for a frame while streaming.
const PARTIAL_OPEN_RE = /<(?:t(?:h(?:i(?:n(?:k(?:i(?:n(?:g)?)?)?)?)?)?)?)?$/i;

function stripTrailingPartialOpen(s: string): string {
  return s.replace(PARTIAL_OPEN_RE, "");
}

export function parseReasoning(input: string): ParsedReasoning {
  if (!input) return { reasoning: "", answer: "", thinking: false };

  const open = OPEN_RE.exec(input);
  if (!open) {
    return { reasoning: "", answer: stripTrailingPartialOpen(input), thinking: false };
  }

  const before = input.slice(0, open.index);
  const afterOpen = input.slice(open.index + open[0].length);
  const close = CLOSE_RE.exec(afterOpen);
  if (!close) {
    // Unclosed — still streaming the reasoning; the answer hasn't started.
    return { reasoning: afterOpen, answer: before, thinking: true };
  }

  const reasoning = afterOpen.slice(0, close.index);
  const rest = afterOpen.slice(close.index + close[0].length);
  return { reasoning, answer: before + stripTrailingPartialOpen(rest), thinking: false };
}

// ---------------------------------------------------------------------------
// Shared expand/collapse state, keyed by reasoning CONTENT (not by row id).
//
// The streaming block (id "streaming") and the finalized history row (id "h<seq>") are two
// different mounts, so per-component `useState` would reset expand state at the finalize swap —
// a visible flash / silent auto-collapse. Keying by a stable hash of the reasoning text (which
// is byte-identical across the swap) makes both mounts read the SAME entry, so an expanded
// disclosure stays expanded through the transition and a collapsed one stays collapsed.
// ---------------------------------------------------------------------------
const expandState = new Map<string, boolean>();
const listeners = new Set<() => void>();
const EXPAND_CAP = 128;

function setExpanded(key: string, val: boolean): void {
  expandState.set(key, val);
  if (expandState.size > EXPAND_CAP) {
    const oldest = expandState.keys().next().value;
    if (oldest !== undefined && oldest !== key) expandState.delete(oldest);
  }
  listeners.forEach((l) => l());
}

export function reasoningKey(reasoning: string): string {
  let h = 5381;
  for (let i = 0; i < reasoning.length; i++) h = ((h << 5) + h + reasoning.charCodeAt(i)) | 0;
  return `r${h}:${reasoning.length}`;
}

export function useReasoningExpanded(key: string): [boolean, () => void] {
  const subscribe = useCallback((cb: () => void) => {
    listeners.add(cb);
    return () => {
      listeners.delete(cb);
    };
  }, []);
  const getSnapshot = useCallback(() => expandState.get(key) ?? false, [key]);
  const expanded = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const toggle = useCallback(() => setExpanded(key, !(expandState.get(key) ?? false)), [key]);
  return [expanded, toggle];
}
