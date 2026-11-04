// The open diff registers how to move through and act on its hunks; the n / p / a / x / c / ?
// shortcuts call it. A module slot rather than context so a shortcut handler never re-renders anything.
export interface DiffController {
  nextHunk: () => void;
  prevHunk: () => void;
  acceptHunk: () => void;
  revertHunk: () => void;
  commentHunk: () => void;
  explainHunk: () => void;
}

let current: DiffController | null = null;

export function registerDiffController(controller: DiffController): () => void {
  current = controller;
  return () => {
    if (current === controller) current = null;
  };
}

export function diffController(): DiffController | null {
  return current;
}
