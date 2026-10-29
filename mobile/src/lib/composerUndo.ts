// Undo/redo for edits the composer makes itself: a voice transcript, an @-mention, an accepted
// suggestion, a prompt pulled back from the queue. Setting a textarea's value from code wipes the
// browser's own undo history, so Ctrl/Cmd+Z could not take these back. Typing keeps the native
// stack; these functions only answer when the draft still equals the result of the last
// programmatic edit.

export interface ProgrammaticEdit {
  before: string;
  after: string;
}

export interface EditStack {
  undo: ProgrammaticEdit[];
  redo: ProgrammaticEdit[];
}

export const EMPTY_EDITS: EditStack = { undo: [], redo: [] };

const MAX_EDITS = 50;

export function recordEdit(stack: EditStack, before: string, after: string): EditStack {
  if (before === after) return stack;
  return { undo: [...stack.undo, { before, after }].slice(-MAX_EDITS), redo: [] };
}

export interface EditResult {
  stack: EditStack;
  text: string;
}

export function undoEdit(stack: EditStack, current: string): EditResult | null {
  const top = stack.undo[stack.undo.length - 1];
  if (!top || top.after !== current) return null;
  return {
    text: top.before,
    stack: { undo: stack.undo.slice(0, -1), redo: [...stack.redo, top] },
  };
}

export function redoEdit(stack: EditStack, current: string): EditResult | null {
  const top = stack.redo[stack.redo.length - 1];
  if (!top || top.before !== current) return null;
  return {
    text: top.after,
    stack: { undo: [...stack.undo, top], redo: stack.redo.slice(0, -1) },
  };
}
