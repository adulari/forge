// A reply that streams for a long time is re-laid-out as Markdown on every frame, and past a few
// thousand lines that dominates the frame. Past `cap` characters only the newest part keeps
// Markdown layout; the older part is shown as plain text until the turn settles, when the whole
// reply renders as Markdown once. The split sits on a paragraph break outside a code fence so the
// Markdown part never starts mid-block.
export const STREAM_MARKDOWN_CAP = 16_000;

export interface StreamSplit {
  /** Older text, shown plain. Empty when the reply is not long enough to need a cap. */
  head: string;
  /** Text that keeps Markdown layout. */
  tail: string;
}

const FENCE = "```";

function fenceCount(text: string): number {
  let count = 0;
  for (let at = text.indexOf(FENCE); at !== -1; at = text.indexOf(FENCE, at + FENCE.length)) count += 1;
  return count;
}

export function splitStreamedText(text: string, streaming: boolean, cap: number = STREAM_MARKDOWN_CAP): StreamSplit {
  if (!streaming || text.length <= cap) return { head: "", tail: text };
  const keep = Math.floor(cap / 2);
  let at = text.lastIndexOf("\n\n", text.length - keep);
  while (at > 0) {
    const head = text.slice(0, at);
    if (fenceCount(head) % 2 === 0) return { head, tail: text.slice(at + 2) };
    at = text.lastIndexOf("\n\n", at - 1);
  }
  return { head: "", tail: text };
}
