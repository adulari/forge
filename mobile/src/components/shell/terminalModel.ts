export const MAX_TERMINALS_PER_SESSION = 8;

function numericSuffix(value: string): number | null {
  const match = /^term-(\d+)$/.exec(value);
  if (!match) return null;
  const number = Number(match[1]);
  return Number.isSafeInteger(number) && number > 0 ? number : null;
}

export function compareTerminalIds(left: string, right: string): number {
  const leftNumber = numericSuffix(left);
  const rightNumber = numericSuffix(right);
  if (leftNumber != null && rightNumber != null) return leftNumber - rightNumber;
  return left.localeCompare(right);
}

export function nextTerminalId(ids: readonly string[]): string | null {
  if (new Set(ids).size >= MAX_TERMINALS_PER_SESSION) return null;
  const used = new Set(ids);
  for (let index = 1; index <= MAX_TERMINALS_PER_SESSION; index += 1) {
    const candidate = `term-${index}`;
    if (!used.has(candidate)) return candidate;
  }
  return null;
}

export function terminalTitle(id: string): string {
  const suffix = numericSuffix(id);
  return suffix == null ? id : `Terminal ${suffix}`;
}

/** DEL — what a pty expects for one character erased. */
export const TERMINAL_BACKSPACE = "\x7f";

/**
 * The hidden capture `TextInput` behind the terminal keeps its own native text buffer, and RN
 * does not guarantee that buffer is actually reset before the next keystroke lands (Android in
 * particular). A naive `onChangeText` handler that sends the WHOLE current text on every change
 * ends up resending everything already sent, every time: typing "echo" one key at a time
 * delivered "e", then "ec", then "ech", then "echo" — sent verbatim each time — which the pty
 * received as "e" + "ec" + "ech" + "echo" = "eecechecho".
 *
 * This computes only what changed since the last buffer contents this function was told about:
 *  - identical text: a no-op event, nothing to send;
 *  - `next` extends `previous` (ordinary typing, whether or not the native buffer actually got
 *    cleared in between): send just the appended tail;
 *  - `next` is a prefix of `previous` (characters were removed — a backspace the input's own
 *    `onKeyPress` didn't catch, which happens on IMEs/keyboards where Android's `onKeyPress`
 *    backspace reporting is unreliable): send one DEL per character removed;
 *  - anything else (autocomplete/predictive text swapping a run of characters, IME composition
 *    replacing text mid-word): there's no safe append/trim relationship to derive keystrokes
 *    from, so `next` is sent as-is — the same "just send what's there" a plain controlled input
 *    already did, now reached only for a genuine non-incremental edit.
 */
export function terminalInputDelta(previous: string, next: string): string {
  if (next === previous) return "";
  if (next.startsWith(previous)) return next.slice(previous.length);
  if (previous.startsWith(next)) return TERMINAL_BACKSPACE.repeat(previous.length - next.length);
  return next;
}

/** A second Ctrl/Cmd+C inside this window after a copy interrupts instead of copying again. */
export const COPY_INTERRUPT_WINDOW_MS = 500;

export type CopyInterruptAction = "copy" | "interrupt" | "ignore";

/**
 * What Ctrl/Cmd+C does in the terminal. With text selected the first press copies it; pressing
 * again within {@link COPY_INTERRUPT_WINDOW_MS} sends the interrupt anyway, so a stray selection
 * never makes a running command impossible to stop. Without a selection Ctrl+C is the interrupt
 * and Cmd+C (which has no terminal meaning) is left alone.
 */
export function copyOrInterrupt(input: {
  meta: boolean;
  hasSelection: boolean;
  now: number;
  lastCopyAt: number | null;
}): CopyInterruptAction {
  if (input.lastCopyAt != null && input.now - input.lastCopyAt <= COPY_INTERRUPT_WINDOW_MS) {
    return "interrupt";
  }
  if (input.hasSelection) return "copy";
  return input.meta ? "ignore" : "interrupt";
}

/** The character grid a pane of this pixel size holds, never smaller than a usable shell. */
export function terminalGrid(input: {
  width: number;
  height: number;
  charWidth: number;
  lineHeight: number;
  paddingX: number;
  minCols?: number;
  minRows?: number;
}): { cols: number; rows: number } {
  const cols = Math.floor((input.width - input.paddingX * 2) / Math.max(input.charWidth, 1));
  const rows = Math.floor(input.height / Math.max(input.lineHeight, 1));
  return { cols: Math.max(input.minCols ?? 20, cols), rows: Math.max(input.minRows ?? 4, rows) };
}

/** Lines above which a resize makes the daemon reflow enough output to be worth waiting out. */
export const LARGE_BUFFER_LINES = 600;
const RESIZE_DEBOUNCE_MS = 160;

/** How long to hold a resize back: dragging a handle floods the pty only when the buffer is big. */
export function resizeDelay(lineCount: number): number {
  return lineCount >= LARGE_BUFFER_LINES ? RESIZE_DEBOUNCE_MS : 0;
}

/** The glyph advance a probe string of `chars` characters measured `width` wide, if it is sane. */
export function measuredCharWidth(width: number, chars: number, fallback: number): number {
  if (!Number.isFinite(width) || chars <= 0) return fallback;
  const each = width / chars;
  return each >= 3 && each <= 40 ? each : fallback;
}

export interface TerminalSummaryLike {
  terminal_id: string;
  status: "running" | "exited";
  sequence?: number;
  exit_code?: number | null;
}

export interface TerminalActivity {
  /** Highest output counter the daemon has reported per terminal. */
  latest: Record<string, number>;
  /** The counter value at the last moment the person was looking at that terminal. */
  seen: Record<string, number>;
  /** Terminals whose counter moved since the previous report. */
  streaming: Record<string, boolean>;
}

export const EMPTY_TERMINAL_ACTIVITY: TerminalActivity = { latest: {}, seen: {}, streaming: {} };

/**
 * Fold a fresh terminal list into the activity record. The terminal being looked at is always
 * "seen"; one met for the first time is too, so output that predates this view is not news.
 */
export function advanceActivity(
  previous: TerminalActivity,
  summaries: readonly TerminalSummaryLike[],
  activeId: string | null,
): TerminalActivity {
  const latest: Record<string, number> = {};
  const seen: Record<string, number> = {};
  const streaming: Record<string, boolean> = {};
  for (const summary of summaries) {
    const id = summary.terminal_id;
    const sequence = summary.sequence ?? 0;
    latest[id] = sequence;
    streaming[id] = id in previous.latest && sequence > previous.latest[id] && summary.status === "running";
    seen[id] = id === activeId || !(id in previous.seen) ? sequence : previous.seen[id];
  }
  return { latest, seen, streaming };
}

/** A background terminal that printed something since it was last looked at. */
export function hasUnseenOutput(activity: TerminalActivity, id: string, activeId: string | null): boolean {
  return id !== activeId && (activity.latest[id] ?? 0) > (activity.seen[id] ?? 0);
}

/** A shell that ended cleanly closes its own tab; any other ending keeps the tab to read and restart. */
export function closesItselfOnExit(exitCode: number | null | undefined): boolean {
  return exitCode === 0;
}

export function exitLabel(exitCode: number | null | undefined): string {
  return exitCode == null ? "exited" : `exited (${exitCode})`;
}

/** The tab to select when `id` goes away: the one that slides into its place, else the one before. */
export function neighbourTerminal(ids: readonly string[], id: string): string | null {
  const index = ids.indexOf(id);
  if (index === -1) return null;
  const rest = ids.filter((candidate) => candidate !== id);
  return rest[Math.min(index, rest.length - 1)] ?? null;
}
