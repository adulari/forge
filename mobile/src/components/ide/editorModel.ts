// The light editor's pure parts: indent, the window of lines worth painting, caret line, and how a
// failed save is explained. The DOM layer in FileEditor.web.tsx only wires these to a textarea.
export const EDITOR_LINE_HEIGHT = 18;
export const INDENT = "  ";

export interface EditResult {
  value: string;
  start: number;
  end: number;
}

/** Tab / Shift+Tab. Without a multi-line selection Tab inserts the indent at the caret. */
export function indentSelection(value: string, start: number, end: number, outdent: boolean): EditResult {
  const startOfLine = value.lastIndexOf("\n", start - 1) + 1;
  const multiline = value.slice(start, end).includes("\n");
  if (!outdent && !multiline) {
    return { value: value.slice(0, start) + INDENT + value.slice(end), start: start + INDENT.length, end: start + INDENT.length };
  }
  // A selection ending right after a newline does not include the line it stops on.
  const blockEnd = end > start && value[end - 1] === "\n" ? end - 1 : end;
  const lastBreak = value.indexOf("\n", blockEnd);
  const blockStop = lastBreak === -1 ? value.length : lastBreak;
  const block = value.slice(startOfLine, blockStop);
  const lines = block.split("\n");
  let firstDelta = 0;
  let totalDelta = 0;
  const changed = lines.map((line, index) => {
    if (!outdent) {
      totalDelta += INDENT.length;
      if (index === 0) firstDelta = INDENT.length;
      return INDENT + line;
    }
    const strip = line.startsWith(INDENT) ? INDENT.length : line.startsWith("\t") ? 1 : line.startsWith(" ") ? 1 : 0;
    totalDelta -= strip;
    if (index === 0) firstDelta = -strip;
    return line.slice(strip);
  });
  return {
    value: value.slice(0, startOfLine) + changed.join("\n") + value.slice(blockStop),
    start: Math.max(startOfLine, start + firstDelta),
    end: Math.max(startOfLine, end + totalDelta),
  };
}

export function lineCountOf(value: string): number {
  let count = 1;
  for (let i = value.indexOf("\n"); i !== -1; i = value.indexOf("\n", i + 1)) count += 1;
  return count;
}

/** Zero-based line of the caret. */
export function lineOfOffset(value: string, offset: number): number {
  let line = 0;
  for (let i = value.indexOf("\n"); i !== -1 && i < offset; i = value.indexOf("\n", i + 1)) line += 1;
  return line;
}

export interface LineWindow {
  first: number;
  last: number;
}

/** Lines [first, last) to paint: the viewport plus overscan, clamped to the file. */
export function lineWindow(scrollTop: number, viewportHeight: number, total: number, overscan = 12, lineHeight = EDITOR_LINE_HEIGHT): LineWindow {
  const first = Math.max(0, Math.floor(scrollTop / lineHeight) - overscan);
  const last = Math.min(total, Math.ceil((scrollTop + viewportHeight) / lineHeight) + overscan);
  return { first, last: Math.max(first, last) };
}

/** scrollTop that puts `line` (zero-based) about a third of the way down the viewport. */
export function scrollTopFor(line: number, viewportHeight: number, total: number, lineHeight = EDITOR_LINE_HEIGHT): number {
  const max = Math.max(0, total * lineHeight - viewportHeight);
  return Math.min(max, Math.max(0, line * lineHeight - viewportHeight / 3));
}

export type SaveFailure = "conflict" | "read_only" | "too_large" | "other";

export function classifySaveError(error: unknown): SaveFailure {
  const status = (error as { status?: unknown } | null)?.status;
  if (status === 409) return "conflict";
  if (status === 413) return "too_large";
  if (status === 403) return "read_only";
  return "other";
}

export type ExternalChange = "none" | "reload" | "conflict";

/** What to do when the file's hash on disk differs from the one the editor loaded. */
export function externalChange(loadedHash: string | null, diskHash: string | undefined, dirty: boolean): ExternalChange {
  if (!diskHash || diskHash === loadedHash) return "none";
  return dirty ? "conflict" : "reload";
}
