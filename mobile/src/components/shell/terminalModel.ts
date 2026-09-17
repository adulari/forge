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
