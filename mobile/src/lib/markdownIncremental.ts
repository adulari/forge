// A streamed reply grows at its end, and `parseBlocks` re-reads the whole of it for every frame.
// The parser is line based and a blank line outside a code fence ends every block, so the text
// before the last such blank line parses to the same blocks whatever follows it. This keeps those
// blocks from frame to frame and parses only the part still being written.
import { type Block, parseBlocks } from "./markdownParse";

export interface IncrementalParse {
  mentions: boolean;
  /** Text already parsed into `blocks`; ends with a newline, right before a blank line. */
  settled: string;
  blocks: Block[];
}

export const EMPTY_INCREMENTAL: IncrementalParse = { mentions: false, settled: "", blocks: [] };

const FENCE_OPEN = /^```\s*([\w-]*)\s*$/;
const FENCE_CLOSE = /^```\s*$/;

/** Where the settled text could grow to: the start of the last blank line that sits outside a
 * code fence and before any display-math marker, scanning `content` from `from` (the start of a
 * line, outside any fence). */
function lastSettledEnd(content: string, from: number): number {
  let end = from;
  let inFence = false;
  let at = from;
  while (at < content.length) {
    const nl = content.indexOf("\n", at);
    const lineEnd = nl === -1 ? content.length : nl;
    const line = content.slice(at, lineEnd);
    if (inFence) {
      if (FENCE_CLOSE.test(line)) inFence = false;
    } else if (FENCE_OPEN.test(line)) {
      inFence = true;
    } else {
      const trimmed = line.trim();
      if (trimmed.startsWith("$$") || trimmed.startsWith("\\[")) break;
      // A blank line with text after it: everything before the line is finished.
      if (trimmed === "" && nl !== -1 && at > from) end = at;
    }
    if (nl === -1) break;
    at = nl + 1;
  }
  return end;
}

/** The blocks of `content`, equal to `parseBlocks(content, mentions)`, reusing what `prev` parsed
 * when `content` only grew. Returns the state to pass in next time. */
export function parseBlocksIncremental(
  content: string,
  mentions: boolean,
  prev: IncrementalParse,
): { blocks: Block[]; state: IncrementalParse } {
  if (content.includes("\r")) {
    return { blocks: parseBlocks(content, mentions), state: EMPTY_INCREMENTAL };
  }
  const reusable = prev.mentions === mentions && prev.settled !== "" && content.startsWith(prev.settled);
  let state: IncrementalParse = reusable ? prev : { mentions, settled: "", blocks: [] };
  const from = state.settled.length;
  const end = lastSettledEnd(content, from);
  if (end > from) {
    const grown = content.slice(0, end);
    const fresh = parseBlocks(content.slice(from, end), mentions);
    state = { mentions, settled: grown, blocks: state.blocks.concat(fresh) };
  }
  const rest = content.slice(state.settled.length);
  const open = rest === "" ? [] : parseBlocks(rest, mentions);
  return { blocks: state.blocks.length ? state.blocks.concat(open) : open, state };
}
