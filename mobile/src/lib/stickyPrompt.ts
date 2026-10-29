// The sticky prompt bar names the user message the top of the visible transcript answers. The
// timeline is newest first (index 0 is the live end), so older is a larger index.

export interface PromptEntry {
  /** Position in the timeline. */
  index: number;
  seq: number;
  text: string;
}

/** The prompt the oldest visible row belongs to, or null when that row is the prompt itself or
 * no prompt lies above the view. `prompts` must be ascending by index. */
export function stickyPromptFor(prompts: readonly PromptEntry[], topIndex: number): PromptEntry | null {
  const owner = prompts.find((prompt) => prompt.index >= topIndex);
  return owner && owner.index > topIndex ? owner : null;
}

/** One line of a prompt: whitespace collapsed, cut with an ellipsis. */
export function promptPreview(text: string, max = 120): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length <= max ? flat : `${flat.slice(0, max - 1).trimEnd()}…`;
}

/** Index of the timeline row that sits `reach` pixels from the newest end (the top edge of the
 * view is `offsetFromNewest + viewportHeight` away from it). Rows not measured yet count as the
 * average of those that are. Heights are tracked per row because a virtualised list's own
 * offsets go stale on web when an earlier row resizes. */
export function rowAtReach(heights: readonly (number | undefined)[], reach: number, fallback = 80): number | null {
  if (heights.length === 0) return null;
  let known = 0;
  let total = 0;
  for (const height of heights) {
    if (height !== undefined) {
      known += 1;
      total += height;
    }
  }
  const guess = known > 0 ? total / known : fallback;
  let covered = 0;
  for (let index = 0; index < heights.length; index++) {
    covered += heights[index] ?? guess;
    if (covered > reach) return index;
  }
  return heights.length - 1;
}
