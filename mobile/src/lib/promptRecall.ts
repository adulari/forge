// Up/Down prompt recall for the web/desktop composer, with the semantics of the TUI's
// `PromptHistory` (crates/forge-tui/src/prompt_history.rs): newest first, consecutive duplicates
// collapsed, any edit ends browsing, and Down past the newest entry restores the draft.
//
// One adaptation: the TUI recalls from any spot on the first row. A web textarea wraps, so Up on
// a long single-line paragraph is a caret move the user expects; recall therefore starts only
// from an empty composer or with the caret at the very start. Once browsing, Up/Down step on the
// first/last row of the recalled prompt as the TUI does.

export interface RecallState {
  /** Index into the entries being browsed, or null when not browsing. */
  pos: number | null;
  /** What the composer held before the first Up. */
  draft: string;
}

export const RECALL_IDLE: RecallState = { pos: null, draft: "" };

export interface RecallStep {
  handled: boolean;
  text: string;
  state: RecallState;
}

/** Trim, drop blanks and back-to-back repeats; oldest first. */
export function collapsePrompts(prompts: readonly string[]): string[] {
  const out: string[] = [];
  for (const raw of prompts) {
    const line = raw.trim();
    if (line && out[out.length - 1] !== line) out.push(line);
  }
  return out;
}

function onFirstRow(text: string, caret: number): boolean {
  return !text.slice(0, caret).includes("\n");
}

function onLastRow(text: string, caret: number): boolean {
  return !text.slice(caret).includes("\n");
}

export function recallStep(
  key: "up" | "down",
  entries: readonly string[],
  state: RecallState,
  text: string,
  caret: number,
  selecting = false,
): RecallStep {
  const pass: RecallStep = { handled: false, text, state };
  if (selecting) return pass;
  if (key === "up") {
    if (entries.length === 0) return pass;
    if (state.pos === null) {
      if (text.length > 0 && caret !== 0) return pass;
      const pos = entries.length - 1;
      return { handled: true, text: entries[pos], state: { pos, draft: text } };
    }
    if (!onFirstRow(text, caret)) return pass;
    const pos = Math.max(0, state.pos - 1);
    return { handled: true, text: entries[pos], state: { ...state, pos } };
  }
  if (state.pos === null || !onLastRow(text, caret)) return pass;
  if (state.pos + 1 < entries.length) {
    const pos = state.pos + 1;
    return { handled: true, text: entries[pos], state: { ...state, pos } };
  }
  return { handled: true, text: state.draft, state: RECALL_IDLE };
}
